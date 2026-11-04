//! A flex item with an `aspect-ratio` and an `auto` height takes its height
//! from the width it ends up with through the ratio, and its content may raise
//! it. That holds for the item of a row container (its height is asked for at the
//! width the container settled, whatever it stretches the item to afterwards), for
//! one of a column container (its main size is asked for at the width it has),
//! and for a paragraph as it does for a block. A container that wraps its items
//! stretches an item only to the line it puts it in, so the width it offers a
//! column item for the stretch is no width for its height.
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
fn a_paragraph_item_of_a_wrapping_row_takes_its_height_from_its_width_along_the_ratio() {
    // The text is 100px wide, so the square is 100px high, though the
    // container is 50px high inside its border and the line is the item's.
    for name in ["base", "flexstart"] {
        check_case(case(name));
    }
}

#[test]
fn a_paragraph_item_wider_than_its_container_is_as_high_as_it_is_wide() {
    for name in ["t15", "t15_basis0", "nw_t15", "block_ctr_t20"] {
        check_case(case(name));
    }
}

#[test]
fn the_height_along_the_ratio_does_not_depend_on_the_alignment_or_the_wrapping() {
    for name in [
        "center",
        "end",
        "baseline",
        "nowrap",
        "autoh",
        "nw_flexstart",
        "nw_center",
        "nw_autoh",
    ] {
        check_case(case(name));
    }
}

#[test]
fn a_paragraph_item_of_a_column_takes_its_main_size_from_its_width_along_the_ratio() {
    for name in [
        "col_nowrap_start",
        "col_nowrap_start_autoh",
        "col_inline_start",
        "col_wrap",
        "col_wrap_t15",
        "col_wrap2",
        "col_wrap2_start",
        "col_wrap2_autoh",
    ] {
        check_case(case(name));
    }
}

#[test]
fn every_case_matches_chromium() {
    for case in CASES {
        check_case(case);
    }
}

/// The cases. A name starts with `nw_` for a container that does not wrap its
/// items, with `w_` or `wrap` for a row that does, with `col` or `c_` for a
/// column, and with `r_` for a row that does not; `t15` is a text of 15
/// characters, and the others say what differs from the case they follow.
#[rustfmt::skip]
const CASES: &[Case] = &[
    ("base", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("flexstart", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;align-items:flex-start;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("center", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;align-items:center;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("end", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;align-items:flex-end;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("baseline", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;align-items:baseline;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("t15", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 100.0)), ("i", (33.0, 33.0, 150.0, 150.0))]),
    ("t15_basis0", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1;flex:0 0 0px">XXXXXXXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 100.0)), ("i", (33.0, 33.0, 150.0, 150.0))]),
    ("t10_basis0", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1;flex:0 0 0px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("t5", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1">XXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 100.0)), ("i", (33.0, 33.0, 50.0, 50.0))]),
    ("nowrap", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 50.0))]),
    ("autoh", r#"<div id=o style="display:inline-flex;;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 150.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("autoh_start", r#"<div id=o style="display:inline-flex;;border:solid 25px green;flex-wrap:wrap;align-items:flex-start;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 150.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("block_child", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1"><div style="width:100px;height:10px"></div></div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("empty_w100", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1;width:100px"></div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("ratio2", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:2/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 50.0))]),
    ("ratio_half", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/2">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 200.0))]),
    ("col_wrap", r#"<div id=o style="display:inline-flex;width:100px;height:100px;border:solid 25px green;flex-wrap:wrap;flex-direction:column;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("col_wrap_t15", r#"<div id=o style="display:inline-flex;width:100px;height:100px;border:solid 25px green;flex-wrap:wrap;flex-direction:column;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 100.0)), ("i", (33.0, 33.0, 150.0, 150.0))]),
    ("wrapfix", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1;width:50px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 100.0)), ("i", (33.0, 33.0, 50.0, 50.0))]),
    ("wrap_h30", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1;height:30px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 30.0))]),
    ("block_ctr", r#"<div id=o style="display:flex;width:150px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("block_ctr_t20", r#"<div id=o style="display:flex;width:150px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1">XXXXXXXXXXXXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (8.0, 8.0, 200.0, 200.0))]),
    ("two_items", r#"<div id=o style="display:flex;width:150px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div><div id=j style="aspect-ratio:1/1">XXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0)), ("j", (108.0, 8.0, 50.0, 100.0))]),
    ("nowrap_block_ctr", r#"<div id=o style="display:flex;width:150px;height:100px"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("nw_flexstart", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("nw_center", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;;align-items:center;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 8.0, 100.0, 100.0))]),
    ("nw_autoh", r#"<div id=o style="display:inline-flex;;border:solid 25px green;;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 150.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("nw_autoh_start", r#"<div id=o style="display:inline-flex;;border:solid 25px green;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 150.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("nw_t15_start", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 100.0)), ("i", (33.0, 33.0, 150.0, 150.0))]),
    ("nw_t15", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 100.0)), ("i", (33.0, 33.0, 150.0, 50.0))]),
    ("nw_t15_autoh", r#"<div id=o style="display:inline-flex;;border:solid 25px green;;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 200.0)), ("i", (33.0, 33.0, 150.0, 150.0))]),
    ("nw_block_start", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;align-items:flex-start"><div id=i style="aspect-ratio:1/1"><div style="width:100px;height:10px"></div></div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("nw_block_t15", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;;"><div id=i style="aspect-ratio:1/1;display:block">XXXXXXXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 100.0)), ("i", (33.0, 33.0, 150.0, 50.0))]),
    ("w_block_start", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;align-items:flex-start"><div id=i style="aspect-ratio:1/1"><div style="width:100px;height:10px"></div></div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 100.0))]),
    ("w_alignstart_2lines", r#"<div id=o style="display:flex;width:100px;height:300px;flex-wrap:wrap;align-items:flex-start"><div id=i style="aspect-ratio:1/1;width:100px">XXXXXXXXXX</div><div id=j style="aspect-ratio:1/1">XXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 300.0)), ("i", (8.0, 8.0, 100.0, 100.0)), ("j", (8.0, 183.0, 50.0, 50.0))]),
    ("w_2lines_auto", r#"<div id=o style="display:flex;width:100px;height:300px;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div><div id=j style="aspect-ratio:1/1">XXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 300.0)), ("i", (8.0, 8.0, 100.0, 175.0)), ("j", (8.0, 183.0, 50.0, 125.0))]),
    ("w_stretch_big", r#"<div id=o style="display:inline-flex;height:300px;border:solid 25px green;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (33.0, 33.0, 100.0, 250.0))]),
    ("w_text_wide", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap"><div id=i style="aspect-ratio:2/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (33.0, 33.0, 100.0, 50.0))]),
    ("w_text_wide_big", r#"<div id=o style="display:inline-flex;height:300px;border:solid 25px green;flex-wrap:wrap"><div id=i style="aspect-ratio:2/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (33.0, 33.0, 100.0, 250.0))]),
    ("w_width_stated", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1;width:60px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 110.0, 100.0)), ("i", (33.0, 33.0, 60.0, 60.0))]),
    ("w_width_stated_short", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1;width:60px">XX</div></div>"#, &[("o", (8.0, 8.0, 110.0, 100.0)), ("i", (33.0, 33.0, 60.0, 60.0))]),
    ("w_short_text", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1">XX</div></div>"#, &[("o", (8.0, 8.0, 70.0, 100.0)), ("i", (33.0, 33.0, 20.0, 50.0))]),
    ("w_grow", r#"<div id=o style="display:flex;width:200px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1;flex-grow:1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 100.0)), ("i", (8.0, 8.0, 200.0, 200.0))]),
    ("w_grow_nowrap", r#"<div id=o style="display:flex;width:200px;height:100px"><div id=i style="aspect-ratio:1/1;flex-grow:1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 100.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("w_shrink", r#"<div id=o style="display:flex;width:60px;height:300px;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1;width:100px;">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 60.0, 300.0)), ("i", (8.0, 8.0, 100.0, 300.0))]),
    ("w_min0", r#"<div id=o style="display:flex;width:60px;height:300px;flex-wrap:wrap"><div id=i style="aspect-ratio:1/1;min-width:0">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 60.0, 300.0)), ("i", (8.0, 8.0, 60.0, 300.0))]),
    ("w_nowrap_min0", r#"<div id=o style="display:flex;width:60px;height:300px;"><div id=i style="aspect-ratio:1/1;min-width:0">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 60.0, 300.0)), ("i", (8.0, 8.0, 60.0, 300.0))]),
    ("col_nowrap", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 150.0, 150.0))]),
    ("col_nowrap_start", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_nowrap_start_autoh", r#"<div id=o style="display:flex;flex-direction:column;width:150px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_nowrap_autoh", r#"<div id=o style="display:flex;flex-direction:column;width:150px;;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 150.0)), ("i", (8.0, 8.0, 150.0, 150.0))]),
    ("col_wrap2", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 150.0, 100.0))]),
    ("col_wrap2_start", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;flex-wrap:wrap;align-items:flex-start;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_wrap2_autoh", r#"<div id=o style="display:flex;flex-direction:column;width:150px;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 100.0)), ("i", (8.0, 8.0, 150.0, 100.0))]),
    ("col_inline_start", r#"<div id=o style="display:inline-flex;flex-direction:column;align-items:flex-start"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_inline_stretch", r#"<div id=o style="display:inline-flex;flex-direction:column"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_inline_t5", r#"<div id=o style="display:inline-flex;flex-direction:column"><div id=i style="aspect-ratio:1/1">XXXXX</div></div>"#, &[("o", (8.0, 8.0, 50.0, 50.0)), ("i", (8.0, 8.0, 50.0, 50.0))]),
    ("col_w_stated", r#"<div id=o style="display:flex;flex-direction:column;width:200px"><div id=i style="aspect-ratio:1/1;width:100px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_h_stated", r#"<div id=o style="display:flex;flex-direction:column;width:200px"><div id=i style="aspect-ratio:1/1;height:100px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 200.0, 100.0)), ("i", (8.0, 8.0, 200.0, 100.0))]),
    ("grid_item", r#"<div id=o style="display:grid;grid-template-columns:150px;"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 784.0, 150.0)), ("i", (8.0, 8.0, 150.0, 150.0))]),
    ("grid_item_start", r#"<div id=o style="display:grid;grid-template-columns:150px;justify-items:start"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 784.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("grid_item_fit", r#"<div id=o style="display:inline-grid"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("block_float", r#"<div id=o style="float:left"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("ib", r#"<div id=o style="display:inline-block"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("row_basis_stated", r#"<div id=o style="display:flex;width:300px"><div id=i style="aspect-ratio:1/1;flex:0 0 100px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("row_grow", r#"<div id=o style="display:flex;width:300px"><div id=i style="aspect-ratio:1/1;flex:1 1 auto;align-self:flex-start">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 300.0)), ("i", (8.0, 8.0, 300.0, 300.0))]),
    ("row_two_start", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1">XXXXXXXXXX</div><div id=j style="aspect-ratio:1/2">XXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0)), ("j", (108.0, 8.0, 50.0, 100.0))]),
    ("row_maxh", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;max-height:40px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 40.0)), ("i", (8.0, 8.0, 40.0, 40.0))]),
    ("row_minh", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;min-height:150px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 150.0)), ("i", (8.0, 8.0, 150.0, 150.0))]),
    ("row_pad", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;padding:10px;box-sizing:border-box">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 120.0)), ("i", (8.0, 8.0, 120.0, 120.0))]),
    ("row_pad_cb", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;padding:10px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 120.0)), ("i", (8.0, 8.0, 120.0, 120.0))]),
    ("row_overflow", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;overflow:hidden">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("row_two_lines_text", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:2/1;width:50px">XXXXX XXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 25.0)), ("i", (8.0, 8.0, 50.0, 25.0))]),
    ("row_inline_children", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1">XX<span style="padding:5px">XX</span>XX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 70.0)), ("i", (8.0, 8.0, 70.0, 70.0))]),
    ("c_maxh", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1;max-height:40px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 40.0, 40.0))]),
    ("c_minh", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1;min-height:150px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 150.0, 150.0))]),
    ("c_pad", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1;padding:10px;box-sizing:border-box">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 120.0, 120.0))]),
    ("c_pad_cb", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1;padding:10px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 120.0, 120.0))]),
    ("c_overflow", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1;overflow:hidden">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("c_maxw", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1;max-width:60px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 60.0, 60.0))]),
    ("c_minw", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1;min-width:150px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 150.0, 150.0))]),
    ("c_shrink", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:40px;;align-items:flex-start;"><div id=i style="aspect-ratio:1/1;">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 40.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("c_wrapped_text", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;;align-items:flex-start;"><div id=i style="aspect-ratio:2/1;width:50px">XXXXX XXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 50.0, 25.0))]),
    ("cw_vert_item", r#"<div id=o style="display:flex;flex-direction:column;width:150px;height:300px;flex-wrap:wrap;"><div id=i style="aspect-ratio:1/1;writing-mode:vertical-rl">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 150.0, 300.0)), ("i", (8.0, 8.0, 150.0, 100.0))]),
    ("r_maxh", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;max-height:40px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 40.0)), ("i", (8.0, 8.0, 40.0, 40.0))]),
    ("r_minw", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;min-width:150px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 150.0)), ("i", (8.0, 8.0, 150.0, 150.0))]),
    ("r_maxw", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;max-width:60px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 60.0)), ("i", (8.0, 8.0, 60.0, 60.0))]),
    ("r_ratio_content_box", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:auto 1/1;padding:10px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 120.0)), ("i", (8.0, 8.0, 120.0, 120.0))]),
    ("r_float_child", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1"><span style="float:left">XX</span>XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 120.0)), ("i", (8.0, 8.0, 120.0, 120.0))]),
    ("r_inline_block", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;display:flow-root">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0))]),
    ("r_margin", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;margin:5px 7px">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 110.0)), ("i", (15.0, 13.0, 100.0, 100.0))]),
    ("r_basis_pct", r#"<div id=o style="display:flex;width:300px;align-items:flex-start"><div id=i style="aspect-ratio:1/1;flex-basis:50%">XXXXXXXXXX</div></div>"#, &[("o", (8.0, 8.0, 300.0, 150.0)), ("i", (8.0, 8.0, 150.0, 150.0))]),
];
