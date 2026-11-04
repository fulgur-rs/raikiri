//! A flex or grid container, or a paragraph, that is an item of a column flex
//! container and has an `aspect-ratio` and an `auto` width takes that width from
//! the height the flex algorithm settled on, whether the item states the height,
//! grows, shrinks or has a flex basis: a `2 / 1` item that flexes to `200px` high
//! is `400px` wide. An item that scrolls, or states a `min-width`, opts out of
//! being raised by its content but still takes the width the ratio gives. A
//! column container that measures its own width has settled no height for its
//! items yet, so such an item adds the width of its content to it, not the one
//! the ratio gives.
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

fn column_items(name: &str) -> bool {
    name.starts_with("a_") && !name.contains("_row_")
}

#[test]
fn a_flex_or_grid_item_of_a_column_takes_its_width_from_the_flexed_height() {
    check_group(column_items);
}

fn row_items(name: &str) -> bool {
    name.starts_with("a_") && name.contains("_row_")
}

#[test]
fn an_item_of_a_row_keeps_taking_its_height_from_the_flexed_width() {
    check_group(row_items);
}

fn scrolling_column_items(name: &str) -> bool {
    name.starts_with("b_") && name.contains("_c_")
}

#[test]
fn a_scrolling_item_of_a_column_takes_its_width_from_the_flexed_height() {
    check_group(scrolling_column_items);
}

fn scrolling_row_items(name: &str) -> bool {
    name.starts_with("b_") && name.contains("_r_")
}

#[test]
fn a_scrolling_item_of_a_row_takes_its_height_from_the_flexed_width() {
    check_group(scrolling_row_items);
}

fn wrapping_columns(name: &str) -> bool {
    name.starts_with("c_") && name.contains("wrapcol")
}

#[test]
fn a_column_that_wraps_gives_its_items_the_same_widths() {
    check_group(wrapping_columns);
}

fn content_and_edges(name: &str) -> bool {
    name.starts_with("c_")
        && ["_wide_", "_pad_", "_r1_", "_r12_", "_margin_", "_center_"]
            .iter()
            .any(|part| name.contains(part))
}

#[test]
fn content_edges_and_alignment_do_not_change_the_width_the_ratio_gives() {
    check_group(content_and_edges);
}

fn item_bounds(name: &str) -> bool {
    name.starts_with("c_")
        && ["_maxw100_", "_maxh50_", "_minh150_"]
            .iter()
            .any(|part| name.contains(part))
}

#[test]
fn the_bounds_of_an_item_apply_to_the_width_the_ratio_gives() {
    check_group(item_bounds);
}

fn several_items(name: &str) -> bool {
    name.starts_with("c_")
        && ["_autoh_", "_two", "_colrev_"]
            .iter()
            .any(|part| name.contains(part))
}

#[test]
fn a_column_of_several_items_or_in_reverse_gives_each_the_same_width() {
    check_group(several_items);
}

fn paragraph_items(name: &str) -> bool {
    name.starts_with("c_para_")
}

#[test]
fn a_paragraph_item_of_a_column_takes_its_width_from_the_flexed_height() {
    check_group(paragraph_items);
}

fn stretched_items(name: &str) -> bool {
    name.starts_with("d_") && name.contains("stretch")
}

#[test]
fn a_stretched_item_takes_the_width_of_the_container() {
    check_group(stretched_items);
}

fn nested_columns(name: &str) -> bool {
    name.starts_with("d_") && name.contains("_nested")
}

#[test]
fn a_column_in_a_column_gives_its_items_the_same_widths() {
    check_group(nested_columns);
}

fn shrink_to_fit_containers(name: &str) -> bool {
    name.starts_with("e_")
        || (name.starts_with("d_")
            && ["_inlineflex_", "_float_", "_abs_", "content"]
                .iter()
                .any(|part| name.contains(part)))
}

#[test]
fn a_container_that_measures_its_own_width_takes_the_content_width_of_its_items() {
    check_group(shrink_to_fit_containers);
}

/// A case that no group selects would never be run.
#[test]
fn every_case_is_in_a_group() {
    let groups: &[fn(&str) -> bool] = &[
        column_items,
        row_items,
        scrolling_column_items,
        scrolling_row_items,
        wrapping_columns,
        content_and_edges,
        item_bounds,
        several_items,
        paragraph_items,
        stretched_items,
        nested_columns,
        shrink_to_fit_containers,
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
    ("a_flex_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_flex_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_flex_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("a_flex_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 1 400px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_flex_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:flex;height:200px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_flex_h200_noshrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:flex;height:200px;flex-shrink:0;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_flex_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex-grow:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_flex_nothing", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:flex;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("a_flex_nothing_h200", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("a_flex_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("a_flex_center_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:center"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-42.0, 8.0, 400.0, 200.0))]),
    ("a_flex_end_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-end"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-92.0, 8.0, 400.0, 200.0))]),
    ("a_flex_flex1_maxh50", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;max-height:50px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_flex_flex1_minh150", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-height:150px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_flex_flex1_maxw100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;max-width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("a_flex_flex1_minw500", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:500px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 500.0, 250.0))]),
    ("a_flex_flex1_w100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("a_flex_h100_ovf", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_flex_flex1_ovf", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_flex_row_w100", r#"<div style="display:flex;width:300px"><div id=i style="display:flex;width:100px;aspect-ratio:2/1"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_flex_row_grow", r#"<div style="display:flex;width:300px"><div id=i style="display:flex;flex-grow:1;aspect-ratio:2/1"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("a_flex_flex1_text", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px">xx xx xx</div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_grid_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_grid_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_grid_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("a_grid_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 1 400px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_grid_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:grid;height:200px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_grid_h200_noshrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:grid;height:200px;flex-shrink:0;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_grid_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex-grow:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_grid_nothing", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:grid;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("a_grid_nothing_h200", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("a_grid_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("a_grid_center_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:center"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-42.0, 8.0, 400.0, 200.0))]),
    ("a_grid_end_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-end"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-92.0, 8.0, 400.0, 200.0))]),
    ("a_grid_flex1_maxh50", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;max-height:50px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_grid_flex1_minh150", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-height:150px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_grid_flex1_maxw100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;max-width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("a_grid_flex1_minw500", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:500px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 500.0, 250.0))]),
    ("a_grid_flex1_w100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("a_grid_h100_ovf", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_grid_flex1_ovf", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_grid_row_w100", r#"<div style="display:flex;width:300px"><div id=i style="display:grid;width:100px;aspect-ratio:2/1"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_grid_row_grow", r#"<div style="display:flex;width:300px"><div id=i style="display:grid;flex-grow:1;aspect-ratio:2/1"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("a_grid_flex1_text", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px">xx xx xx</div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_block_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:block;height:100px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_block_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_block_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("a_block_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:0 1 400px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_block_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:block;height:200px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_block_h200_noshrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:block;height:200px;flex-shrink:0;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_block_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex-grow:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_block_nothing", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:block;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("a_block_nothing_h200", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("a_block_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:block;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("a_block_center_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:center"><div id=i style="display:block;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-42.0, 8.0, 400.0, 200.0))]),
    ("a_block_end_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-end"><div id=i style="display:block;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-92.0, 8.0, 400.0, 200.0))]),
    ("a_block_flex1_maxh50", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;max-height:50px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_block_flex1_minh150", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;min-height:150px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_block_flex1_maxw100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;max-width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("a_block_flex1_minw500", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;min-width:500px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 500.0, 250.0))]),
    ("a_block_flex1_w100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("a_block_h100_ovf", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:block;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_block_flex1_ovf", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("a_block_row_w100", r#"<div style="display:flex;width:300px"><div id=i style="display:block;width:100px;aspect-ratio:2/1"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_block_row_grow", r#"<div style="display:flex;width:300px"><div id=i style="display:block;flex-grow:1;aspect-ratio:2/1"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("a_block_flex1_text", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px">xx xx xx</div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e0_c_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_flex_e0_c_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e0_c_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("b_flex_e0_c_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 1 400px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e0_c_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:flex;height:200px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_flex_e0_c_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex-grow:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e0_c_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("b_flex_e0_r_w100", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:flex;width:100px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("b_flex_e0_r_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_flex_e0_r_basis80", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 80.0, 160.0))]),
    ("b_flex_e0_r_shrink400", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 1 400px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_flex_e0_r_w400_shrink", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:flex;width:400px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_flex_e0_r_stretch_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;"><div id=i style="display:flex;flex:1;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_flex_e2_c_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_flex_e2_c_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e2_c_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("b_flex_e2_c_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 1 400px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e2_c_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:flex;height:200px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_flex_e2_c_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex-grow:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e2_c_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("b_flex_e2_r_w100", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:flex;width:100px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("b_flex_e2_r_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_flex_e2_r_basis80", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 80.0, 160.0))]),
    ("b_flex_e2_r_shrink400", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 1 400px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_flex_e2_r_w400_shrink", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:flex;width:400px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_flex_e2_r_stretch_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;"><div id=i style="display:flex;flex:1;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_flex_e4_c_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_flex_e4_c_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e4_c_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("b_flex_e4_c_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 1 400px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e4_c_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:flex;height:200px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_flex_e4_c_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex-grow:1;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_flex_e4_c_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("b_flex_e4_r_w100", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:flex;width:100px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("b_flex_e4_r_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_flex_e4_r_basis80", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 80.0, 160.0))]),
    ("b_flex_e4_r_shrink400", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 1 400px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_flex_e4_r_w400_shrink", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:flex;width:400px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_flex_e4_r_stretch_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;"><div id=i style="display:flex;flex:1;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_grid_e0_c_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_grid_e0_c_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_grid_e0_c_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("b_grid_e0_c_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 1 400px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_grid_e0_c_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:grid;height:200px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_grid_e0_c_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex-grow:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_grid_e0_c_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("b_grid_e0_r_w100", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:grid;width:100px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("b_grid_e0_r_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_grid_e0_r_basis80", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 80.0, 160.0))]),
    ("b_grid_e0_r_shrink400", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 1 400px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_grid_e0_r_w400_shrink", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:grid;width:400px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_grid_e0_r_stretch_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;"><div id=i style="display:grid;flex:1;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_grid_e2_c_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_grid_e2_c_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_grid_e2_c_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("b_grid_e2_c_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 1 400px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_grid_e2_c_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:grid;height:200px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_grid_e2_c_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex-grow:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_grid_e2_c_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("b_grid_e2_r_w100", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:grid;width:100px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("b_grid_e2_r_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_grid_e2_r_basis80", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 80.0, 160.0))]),
    ("b_grid_e2_r_shrink400", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 1 400px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_grid_e2_r_w400_shrink", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:grid;width:400px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_grid_e2_r_stretch_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;"><div id=i style="display:grid;flex:1;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_grid_e4_c_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_grid_e4_c_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_grid_e4_c_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("b_grid_e4_c_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 1 400px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_grid_e4_c_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:grid;height:200px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_grid_e4_c_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex-grow:1;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_grid_e4_c_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("b_grid_e4_r_w100", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:grid;width:100px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("b_grid_e4_r_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_grid_e4_r_basis80", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 80.0, 160.0))]),
    ("b_grid_e4_r_shrink400", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 1 400px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_grid_e4_r_w400_shrink", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:grid;width:400px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_grid_e4_r_stretch_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;"><div id=i style="display:grid;flex:1;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_block_e0_c_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:block;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_block_e0_c_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_block_e0_c_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("b_block_e0_c_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:0 1 400px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_block_e0_c_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:block;height:200px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_block_e0_c_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex-grow:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_block_e0_c_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:block;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("b_block_e0_r_w100", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:block;width:100px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("b_block_e0_r_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_block_e0_r_basis80", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:block;flex:0 0 80px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 80.0, 160.0))]),
    ("b_block_e0_r_shrink400", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:block;flex:0 1 400px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_block_e0_r_w400_shrink", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:block;width:400px;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_block_e0_r_stretch_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;"><div id=i style="display:block;flex:1;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_block_e2_c_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:block;height:100px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_block_e2_c_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_block_e2_c_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("b_block_e2_c_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:0 1 400px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_block_e2_c_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:block;height:200px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_block_e2_c_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex-grow:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_block_e2_c_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:block;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("b_block_e2_r_w100", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:block;width:100px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("b_block_e2_r_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_block_e2_r_basis80", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:block;flex:0 0 80px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 80.0, 160.0))]),
    ("b_block_e2_r_shrink400", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:block;flex:0 1 400px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_block_e2_r_w400_shrink", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:block;width:400px;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_block_e2_r_stretch_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;"><div id=i style="display:block;flex:1;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_block_e4_c_h100", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start"><div id=i style="display:block;height:100px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_block_e4_c_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_block_e4_c_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:0 0 80px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("b_block_e4_c_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex:0 1 400px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_block_e4_c_h200_shrink", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;align-items:flex-start"><div id=i style="display:block;height:200px;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("b_block_e4_c_grow_auto", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:block;flex-grow:1;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("b_block_e4_c_stretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id=i style="display:block;flex:1;aspect-ratio:2/1;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("b_block_e4_r_w100", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:block;width:100px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("b_block_e4_r_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:block;flex:1;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("b_block_e4_r_basis80", r#"<div style="display:flex;flex-direction:row;width:250px;align-items:flex-start"><div id=i style="display:block;flex:0 0 80px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 80.0, 160.0))]),
    ("b_block_e4_r_shrink400", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:block;flex:0 1 400px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_block_e4_r_w400_shrink", r#"<div style="display:flex;flex-direction:row;width:200px;align-items:flex-start"><div id=i style="display:block;width:400px;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 400.0))]),
    ("b_block_e4_r_stretch_flex1", r#"<div style="display:flex;flex-direction:row;width:250px;"><div id=i style="display:block;flex:1;aspect-ratio:1/2;min-width:10px;min-height:10px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 250.0, 500.0))]),
    ("c_flex_x0_wrapcol_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x0_wrapcol_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_flex_x0_wide_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x0_wide_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_flex_x0_maxw100_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden;max-width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_flex_x0_maxh50_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden;max-height:50px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("c_flex_x0_minh150_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden;min-height:150px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_flex_x0_autoh_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_flex_x0_autoh_grow", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 0.0, 0.0))]),
    ("c_flex_x0_pad_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden;padding:10px;border:5px solid"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x0_r1_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("c_flex_x0_r12_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_flex_x0_margin_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden;margin:5px 7px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (15.0, 13.0, 380.0, 190.0))]),
    ("c_flex_x0_center_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:center;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-42.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x0_two", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div><div id=j style="display:flex;flex:1;aspect-ratio:3/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0)), ("j", (8.0, 158.0, 450.0, 150.0))]),
    ("c_flex_x0_colrev_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-direction:column-reverse"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x1_wrapcol_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x1_wrapcol_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_flex_x1_wide_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x1_wide_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_flex_x1_maxw100_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0;max-width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_flex_x1_maxh50_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0;max-height:50px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("c_flex_x1_minh150_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0;min-height:150px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_flex_x1_autoh_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_flex_x1_autoh_grow", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("c_flex_x1_pad_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0;padding:10px;border:5px solid"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x1_r1_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("c_flex_x1_r12_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_flex_x1_margin_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0;margin:5px 7px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (15.0, 13.0, 380.0, 190.0))]),
    ("c_flex_x1_center_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:center;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-42.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x1_two", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div><div id=j style="display:flex;flex:1;aspect-ratio:3/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0)), ("j", (8.0, 158.0, 450.0, 150.0))]),
    ("c_flex_x1_colrev_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-direction:column-reverse"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x2_wrapcol_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x2_wrapcol_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_flex_x2_wide_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x2_wide_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_flex_x2_maxw100_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;;max-width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_flex_x2_maxh50_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;;max-height:50px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("c_flex_x2_minh150_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;;min-height:150px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_flex_x2_autoh_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_flex_x2_autoh_grow", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("c_flex_x2_pad_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;;padding:10px;border:5px solid"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x2_r1_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("c_flex_x2_r12_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:1/2;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_flex_x2_margin_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;;margin:5px 7px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (15.0, 13.0, 380.0, 190.0))]),
    ("c_flex_x2_center_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:center;"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-42.0, 8.0, 400.0, 200.0))]),
    ("c_flex_x2_two", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div><div id=j style="display:flex;flex:1;aspect-ratio:3/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0)), ("j", (8.0, 158.0, 450.0, 150.0))]),
    ("c_flex_x2_colrev_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-direction:column-reverse"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x0_wrapcol_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x0_wrapcol_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_grid_x0_wide_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x0_wide_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_grid_x0_maxw100_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden;max-width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_grid_x0_maxh50_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden;max-height:50px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("c_grid_x0_minh150_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden;min-height:150px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_grid_x0_autoh_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_grid_x0_autoh_grow", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 0.0, 0.0))]),
    ("c_grid_x0_pad_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden;padding:10px;border:5px solid"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x0_r1_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("c_grid_x0_r12_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:1/2;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_grid_x0_margin_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden;margin:5px 7px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (15.0, 13.0, 380.0, 190.0))]),
    ("c_grid_x0_center_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:center;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-42.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x0_two", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div><div id=j style="display:grid;flex:1;aspect-ratio:3/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0)), ("j", (8.0, 158.0, 450.0, 150.0))]),
    ("c_grid_x0_colrev_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-direction:column-reverse"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x1_wrapcol_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x1_wrapcol_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_grid_x1_wide_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x1_wide_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_grid_x1_maxw100_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0;max-width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_grid_x1_maxh50_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0;max-height:50px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("c_grid_x1_minh150_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0;min-height:150px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_grid_x1_autoh_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_grid_x1_autoh_grow", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("c_grid_x1_pad_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0;padding:10px;border:5px solid"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x1_r1_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("c_grid_x1_r12_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:1/2;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_grid_x1_margin_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0;margin:5px 7px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (15.0, 13.0, 380.0, 190.0))]),
    ("c_grid_x1_center_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:center;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-42.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x1_two", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div><div id=j style="display:grid;flex:1;aspect-ratio:3/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0)), ("j", (8.0, 158.0, 450.0, 150.0))]),
    ("c_grid_x1_colrev_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-direction:column-reverse"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x2_wrapcol_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x2_wrapcol_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-wrap:wrap"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_grid_x2_wide_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x2_wide_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:300px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_grid_x2_maxw100_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;;max-width:100px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_grid_x2_maxh50_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;;max-height:50px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("c_grid_x2_minh150_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;;min-height:150px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("c_grid_x2_autoh_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_grid_x2_autoh_grow", r#"<div style="display:flex;flex-direction:column;width:300px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 40.0, 20.0))]),
    ("c_grid_x2_pad_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;;padding:10px;border:5px solid"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x2_r1_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("c_grid_x2_r12_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:1/2;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("c_grid_x2_margin_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;;margin:5px 7px"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (15.0, 13.0, 380.0, 190.0))]),
    ("c_grid_x2_center_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:center;"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (-42.0, 8.0, 400.0, 200.0))]),
    ("c_grid_x2_two", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div><div id=j style="display:grid;flex:1;aspect-ratio:3/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0)), ("j", (8.0, 158.0, 450.0, 150.0))]),
    ("c_grid_x2_colrev_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;flex-direction:column-reverse"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_para_x0_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:1;aspect-ratio:2/1;overflow:hidden">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_para_x0_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:0 0 80px;aspect-ratio:2/1;overflow:hidden">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_para_x0_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:0 1 400px;aspect-ratio:2/1;overflow:hidden">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_para_x0_flex1_ib", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:1;display:inline-block;aspect-ratio:2/1;overflow:hidden">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_para_x1_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:1;aspect-ratio:2/1;min-width:0">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_para_x1_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:0 0 80px;aspect-ratio:2/1;min-width:0">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_para_x1_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:0 1 400px;aspect-ratio:2/1;min-width:0">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_para_x1_flex1_ib", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:1;display:inline-block;aspect-ratio:2/1;min-width:0">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_para_x2_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:1;aspect-ratio:2/1;">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_para_x2_basis80", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:0 0 80px;aspect-ratio:2/1;">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("c_para_x2_shrink400", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:0 1 400px;aspect-ratio:2/1;">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("c_para_x2_flex1_ib", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start;"><div id=i style="flex:1;display:inline-block;aspect-ratio:2/1;">xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s0_selfstretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;align-self:stretch;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("d_flex_s0_selfstretch_h100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;height:100px;align-self:stretch;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 100.0))]),
    ("d_flex_s0_inlineflex_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s0_inlineflex_basis80", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("d_flex_s0_inlineflex_h100", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("d_flex_s0_float_flex1", r#"<div style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s0_abs_flex1", r#"<div style="position:absolute;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s0_nested", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div style="display:flex;flex-direction:column;flex:1;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div></div>"#, &[("i", (8.0, 8.0, 600.0, 300.0))]),
    ("d_flex_s0_mincontent", r#"<div style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s0_maxcontent", r#"<div style="display:flex;flex-direction:column;width:max-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s0_fitcontent", r#"<div style="display:flex;flex-direction:column;width:fit-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s1_selfstretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;align-self:stretch;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("d_flex_s1_selfstretch_h100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;height:100px;align-self:stretch;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 100.0))]),
    ("d_flex_s1_inlineflex_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s1_inlineflex_stretch_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 0.0, 200.0))]),
    ("d_flex_s1_inlineflex_basis80", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("d_flex_s1_inlineflex_h100", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("d_flex_s1_float_flex1", r#"<div style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s1_abs_flex1", r#"<div style="position:absolute;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s1_nested", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div style="display:flex;flex-direction:column;flex:1;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div></div>"#, &[("i", (8.0, 8.0, 600.0, 300.0))]),
    ("d_flex_s1_mincontent", r#"<div style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s1_maxcontent", r#"<div style="display:flex;flex-direction:column;width:max-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s1_fitcontent", r#"<div style="display:flex;flex-direction:column;width:fit-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s2_selfstretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;align-self:stretch;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("d_flex_s2_selfstretch_h100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:flex;height:100px;align-self:stretch;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 100.0))]),
    ("d_flex_s2_inlineflex_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s2_inlineflex_stretch_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 0.0, 200.0))]),
    ("d_flex_s2_inlineflex_basis80", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("d_flex_s2_inlineflex_h100", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("d_flex_s2_float_flex1", r#"<div style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s2_abs_flex1", r#"<div style="position:absolute;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s2_nested", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div style="display:flex;flex-direction:column;flex:1;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div></div>"#, &[("i", (8.0, 8.0, 600.0, 300.0))]),
    ("d_flex_s2_mincontent", r#"<div style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s2_maxcontent", r#"<div style="display:flex;flex-direction:column;width:max-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_flex_s2_fitcontent", r#"<div style="display:flex;flex-direction:column;width:fit-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s0_selfstretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;align-self:stretch;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("d_grid_s0_selfstretch_h100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;height:100px;align-self:stretch;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 100.0))]),
    ("d_grid_s0_inlineflex_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s0_inlineflex_basis80", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("d_grid_s0_inlineflex_h100", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("d_grid_s0_float_flex1", r#"<div style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s0_abs_flex1", r#"<div style="position:absolute;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s0_nested", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div style="display:flex;flex-direction:column;flex:1;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div></div>"#, &[("i", (8.0, 8.0, 600.0, 300.0))]),
    ("d_grid_s0_mincontent", r#"<div style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s0_maxcontent", r#"<div style="display:flex;flex-direction:column;width:max-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s0_fitcontent", r#"<div style="display:flex;flex-direction:column;width:fit-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s1_selfstretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;align-self:stretch;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("d_grid_s1_selfstretch_h100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;height:100px;align-self:stretch;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 100.0))]),
    ("d_grid_s1_inlineflex_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s1_inlineflex_stretch_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 0.0, 200.0))]),
    ("d_grid_s1_inlineflex_basis80", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("d_grid_s1_inlineflex_h100", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("d_grid_s1_float_flex1", r#"<div style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s1_abs_flex1", r#"<div style="position:absolute;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s1_nested", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div style="display:flex;flex-direction:column;flex:1;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div></div>"#, &[("i", (8.0, 8.0, 600.0, 300.0))]),
    ("d_grid_s1_mincontent", r#"<div style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s1_maxcontent", r#"<div style="display:flex;flex-direction:column;width:max-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s1_fitcontent", r#"<div style="display:flex;flex-direction:column;width:fit-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s2_selfstretch_flex1", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;align-self:stretch;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 200.0))]),
    ("d_grid_s2_selfstretch_h100", r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;align-items:flex-start"><div id=i style="display:grid;height:100px;align-self:stretch;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 100.0))]),
    ("d_grid_s2_inlineflex_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s2_inlineflex_stretch_flex1", r#"<div style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 0.0, 200.0))]),
    ("d_grid_s2_inlineflex_basis80", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 160.0, 80.0))]),
    ("d_grid_s2_inlineflex_h100", r#"<div style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("d_grid_s2_float_flex1", r#"<div style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s2_abs_flex1", r#"<div style="position:absolute;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s2_nested", r#"<div style="display:flex;flex-direction:column;width:300px;height:300px;align-items:flex-start"><div style="display:flex;flex-direction:column;flex:1;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div></div>"#, &[("i", (8.0, 8.0, 600.0, 300.0))]),
    ("d_grid_s2_mincontent", r#"<div style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s2_maxcontent", r#"<div style="display:flex;flex-direction:column;width:max-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("d_grid_s2_fitcontent", r#"<div style="display:flex;flex-direction:column;width:fit-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_flex_s0_inlineflex_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_flex_s0_inlineflex_stretch_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_flex_s1_inlineflex_flex1", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_flex_s1_inlineflex_stretch_flex1", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 200.0))]),
    ("e_flex_s1_inlineflex_basis80", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 160.0, 80.0))]),
    ("e_flex_s1_inlineflex_stretch_basis80", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_flex_s1_inlineflex_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_flex_s1_inlineflex_stretch_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_flex_s1_float_flex1", r#"<div id=c style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_flex_s1_abs_basis80", r#"<div id=c style="position:absolute;display:flex;flex-direction:column;height:200px"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_flex_s1_mincontent_flex1", r#"<div id=c style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_flex_s1_fitcontent_stretch_basis80", r#"<div id=c style="display:flex;flex-direction:column;width:fit-content;height:200px"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_flex_s2_inlineflex_flex1", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_flex_s2_inlineflex_stretch_flex1", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 200.0))]),
    ("e_flex_s2_inlineflex_basis80", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 160.0, 80.0))]),
    ("e_flex_s2_inlineflex_stretch_basis80", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_flex_s2_inlineflex_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_flex_s2_inlineflex_stretch_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:flex;height:100px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_flex_s2_float_flex1", r#"<div id=c style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_flex_s2_abs_basis80", r#"<div id=c style="position:absolute;display:flex;flex-direction:column;height:200px"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_flex_s2_mincontent_flex1", r#"<div id=c style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:flex;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_flex_s2_fitcontent_stretch_basis80", r#"<div id=c style="display:flex;flex-direction:column;width:fit-content;height:200px"><div id=i style="display:flex;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_grid_s0_inlineflex_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_grid_s0_inlineflex_stretch_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_grid_s1_inlineflex_flex1", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_grid_s1_inlineflex_stretch_flex1", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 200.0))]),
    ("e_grid_s1_inlineflex_basis80", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 160.0, 80.0))]),
    ("e_grid_s1_inlineflex_stretch_basis80", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_grid_s1_inlineflex_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_grid_s1_inlineflex_stretch_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_grid_s1_float_flex1", r#"<div id=c style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_grid_s1_abs_basis80", r#"<div id=c style="position:absolute;display:flex;flex-direction:column;height:200px"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_grid_s1_mincontent_flex1", r#"<div id=c style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_grid_s1_fitcontent_stretch_basis80", r#"<div id=c style="display:flex;flex-direction:column;width:fit-content;height:200px"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;overflow:hidden"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_grid_s2_inlineflex_flex1", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_grid_s2_inlineflex_stretch_flex1", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 200.0))]),
    ("e_grid_s2_inlineflex_basis80", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 160.0, 80.0))]),
    ("e_grid_s2_inlineflex_stretch_basis80", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_grid_s2_inlineflex_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_grid_s2_inlineflex_stretch_h100", r#"<div id=c style="display:inline-flex;flex-direction:column;height:200px"><div id=i style="display:grid;height:100px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("e_grid_s2_float_flex1", r#"<div id=c style="float:left;display:flex;flex-direction:column;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_grid_s2_abs_basis80", r#"<div id=c style="position:absolute;display:flex;flex-direction:column;height:200px"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
    ("e_grid_s2_mincontent_flex1", r#"<div id=c style="display:flex;flex-direction:column;width:min-content;height:200px;align-items:flex-start"><div id=i style="display:grid;flex:1;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 400.0, 200.0))]),
    ("e_grid_s2_fitcontent_stretch_basis80", r#"<div id=c style="display:flex;flex-direction:column;width:fit-content;height:200px"><div id=i style="display:grid;flex:0 0 80px;aspect-ratio:2/1;min-width:0"><div style="width:100%;height:20px"></div></div></div>"#, &[("c", (8.0, 8.0, 0.0, 200.0)), ("i", (8.0, 8.0, 0.0, 80.0))]),
];
