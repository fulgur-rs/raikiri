//! Flex containers in vertical writing modes lay out on the axes of their own
//! writing mode: the main axis of a row is the inline axis, the physical
//! height, and that of a column is the block axis, the physical width.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border boxes `(x, y, width, height)` of the elements of `body` with an
/// `id`, in the order they were found on the first page.
fn border_boxes(body: &str) -> Vec<(String, (f32, f32, f32, f32))> {
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
    let page = result.page(0).expect("page");
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

fn check(body: &str, expected: &[(&str, (f32, f32, f32, f32))]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

#[test]
fn vertical_rl_row_flex_runs_from_the_top_right() {
    // In `vertical-rl` the main axis of a row is the inline axis, from top to
    // bottom, and the cross axis starts at the right edge. `vertical-lr`
    // starts the cross axis at the left edge.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 8.0, 30.0, 20.0)),
            ("b", (268.0, 28.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-lr;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 30.0, 20.0)),
            ("b", (8.0, 28.0, 40.0, 50.0)),
        ],
    );
}

#[test]
fn vertical_column_flex_stacks_items_along_the_block_axis() {
    // A column container in a vertical writing mode stacks its items along the
    // block axis: from right to left in `vertical-rl`, from left to right in
    // `vertical-lr`.
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 8.0, 30.0, 20.0)),
            ("b", (238.0, 8.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-lr;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 30.0, 20.0)),
            ("b", (38.0, 8.0, 40.0, 50.0)),
        ],
    );
}

#[test]
fn vertical_flex_follows_the_inline_direction() {
    // `row-reverse` and `direction: rtl` start the inline axis at the bottom;
    // `direction` also reverses the cross start of a wrapping column
    // container.
    check(
        r#"<div id=c style="display:flex;flex-direction:row-reverse;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 188.0, 30.0, 20.0)),
            ("b", (268.0, 138.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;direction:rtl;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 188.0, 30.0, 20.0)),
            ("b", (268.0, 138.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;direction:rtl;writing-mode:vertical-lr;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 188.0, 30.0, 20.0)),
            ("b", (38.0, 158.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;direction:rtl;flex-direction:column;flex-wrap:wrap;writing-mode:vertical-rl;width:300px;height:100px"><div id=a style="width:30px;height:60px"></div><div id=b style="width:40px;height:50px"></div><div id=d style="width:20px;height:30px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (278.0, 48.0, 30.0, 60.0)),
            ("b", (238.0, 58.0, 40.0, 50.0)),
            ("d", (218.0, 78.0, 20.0, 30.0)),
        ],
    );
}

#[test]
fn vertical_flex_wraps_and_gaps_on_its_own_axes() {
    // `flex-wrap` breaks lines at the physical height of a row container and
    // at the physical width of a column one, and `gap` is `row-gap` along the
    // lines.
    check(
        r#"<div id=c style="display:flex;flex-wrap:wrap;gap:7px 5px;writing-mode:vertical-rl;width:300px;height:100px"><div id=a style="width:30px;height:60px"></div><div id=b style="width:40px;height:50px"></div><div id=d style="width:20px;height:30px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (278.0, 8.0, 30.0, 60.0)),
            ("b", (119.5, 8.0, 40.0, 50.0)),
            ("d", (139.5, 63.0, 20.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;writing-mode:vertical-rl;width:200px;height:100px"><div id=a style="width:30px;height:60px"></div><div id=b style="width:40px;height:50px"></div><div id=d style="width:20px;height:30px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (178.0, 8.0, 30.0, 60.0)),
            ("b", (138.0, 8.0, 40.0, 50.0)),
            ("d", (118.0, 8.0, 20.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-wrap:wrap;align-content:space-around;writing-mode:vertical-rl;width:300px;height:100px"><div id=a style="width:30px;height:60px"></div><div id=b style="width:40px;height:50px"></div><div id=d style="width:20px;height:30px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (220.5, 8.0, 30.0, 60.0)),
            ("b", (65.5, 8.0, 40.0, 50.0)),
            ("d", (85.5, 58.0, 20.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-wrap:wrap-reverse;writing-mode:vertical-rl;width:300px;height:100px"><div id=a style="width:30px;height:60px"></div><div id=b style="width:40px;height:50px"></div><div id=d style="width:20px;height:30px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (8.0, 8.0, 30.0, 60.0)),
            ("b", (153.0, 8.0, 40.0, 50.0)),
            ("d", (153.0, 58.0, 20.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-wrap:wrap-reverse;writing-mode:vertical-lr;width:300px;height:100px"><div id=a style="width:30px;height:60px"></div><div id=b style="width:40px;height:50px"></div><div id=d style="width:20px;height:30px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (278.0, 8.0, 30.0, 60.0)),
            ("b", (123.0, 8.0, 40.0, 50.0)),
            ("d", (143.0, 58.0, 20.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-wrap:wrap;writing-mode:vertical-rl;width:300px;height:100px"><div id=a style="height:60px">XXX</div><div id=b style="height:50px">XX</div><div id=d style="height:30px">X</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (158.0, 8.0, 150.0, 60.0)),
            ("b", (8.0, 8.0, 150.0, 50.0)),
            ("d", (8.0, 58.0, 150.0, 30.0)),
        ],
    );
}

#[test]
fn vertical_flex_distributes_free_space_along_the_inline_axis() {
    // Growing, shrinking and `justify-content` act on the physical height of a
    // row container.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:100px"><div id=a style="width:30px;height:80px;flex:1 1 auto"></div><div id=b style="width:40px;height:80px;flex:2 1 0"></div><div id=d style="width:20px;flex:0 1 80px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (278.0, 8.0, 30.0, 50.0)),
            ("b", (268.0, 58.0, 40.0, 0.0)),
            ("d", (288.0, 58.0, 20.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;justify-content:space-between;align-content:center;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div><div id=d style="width:10px;height:10px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 8.0, 30.0, 20.0)),
            ("b", (268.0, 88.0, 40.0, 50.0)),
            ("d", (298.0, 198.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:200px;height:100px"><div id=a style="flex:1">XXX</div><div id=b style="flex:2">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (141.3281, 8.0, 66.6719, 100.0)),
            ("b", (8.0, 8.0, 133.3281, 100.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:100px;height:100px"><div id=a style="width:80px">XXX</div><div id=b style="width:80px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 100.0, 100.0)),
            ("a", (58.0, 8.0, 50.0, 100.0)),
            ("b", (8.0, 8.0, 50.0, 100.0)),
        ],
    );
}

#[test]
fn vertical_flex_aligns_items_across_the_block_axis() {
    // `align-items` and `align-self` act on the physical width of a row
    // container, and `auto` margins absorb free space on both axes.
    check(
        r#"<div id=c style="display:flex;align-items:center;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px;align-self:flex-end"></div><div id=d style="width:10px;height:50px;align-self:stretch"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (143.0, 8.0, 30.0, 20.0)),
            ("b", (8.0, 28.0, 40.0, 50.0)),
            ("d", (298.0, 78.0, 10.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px;margin:auto"></div><div id=b style="width:40px;height:50px;margin-top:auto"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (143.0, 51.3281, 30.0, 20.0)),
            ("b", (268.0, 157.9844, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px;order:2"></div><div id=b style="width:40px;height:50px;order:1"></div><div id=d style="display:none"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 58.0, 30.0, 20.0)),
            ("b", (268.0, 8.0, 40.0, 50.0)),
        ],
    );
}

#[test]
fn vertical_flex_resolves_edges_against_its_own_inline_size() {
    // Margins, paddings and borders of the container and its items resolve
    // against the inline size of the containing block, the physical height of
    // a vertical container, also for items without children.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px;padding:3px 5px 7px 11px;border:2px solid"><div id=a style="width:30px;height:20px;margin:1px 2px 3px 4px"></div><div id=b style="width:40px;height:50px;margin:auto 0"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (269.0, 14.0, 30.0, 20.0)),
            ("b", (261.0, 93.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px;padding:10%"></div><div id=b style="width:40px;height:50px;margin-left:5%"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (268.0, 8.0, 40.0, 40.0)),
            ("b", (268.0, 48.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px;padding:10%"><div id=x style="width:5px;height:5px"></div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (268.0, 8.0, 40.0, 40.0)),
            ("x", (283.0, 28.0, 5.0, 5.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px;margin:5% 10% 5% 10%"></div><div id=b style="width:30px;height:20px;margin:2px"><div style="width:1px;height:1px"></div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (258.0, 18.0, 30.0, 20.0)),
            ("b", (276.0, 50.0, 30.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;gap:10%;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 8.0, 30.0, 20.0)),
            ("b", (268.0, 48.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;gap:10%;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:30px;height:20px"></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 8.0, 30.0, 20.0)),
            ("b", (208.0, 8.0, 40.0, 50.0)),
        ],
    );
}

#[test]
fn vertical_flex_items_size_by_content_on_both_axes() {
    // An item without a size takes its content as its main size and the line
    // as its cross size.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a><div style="width:30px;height:20px"></div></div><div id=b><div style="width:40px;height:50px"></div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 20.0)),
            ("b", (8.0, 28.0, 300.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a>XXXXX</div><div id=b>XXX XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 50.0)),
            ("b", (8.0, 58.0, 300.0, 70.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-lr;width:300px;height:100px"><div id=a>XXXXX</div><div id=b>XXX XXX XXX XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (8.0, 8.0, 300.0, 50.0)),
            ("b", (8.0, 58.0, 300.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:100px"><div id=a>XXXXX</div><div id=b>XXX XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 20.0, 100.0)),
            ("a", (18.0, 8.0, 10.0, 100.0)),
            ("b", (8.0, 8.0, 10.0, 100.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a>XX<span id=s style="display:inline-block;width:10px;height:15px"></span>XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 55.0)),
            ("s", (298.0, 28.0, 10.0, 15.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:60px"><div id=f style="float:left;width:10px;height:10px"></div>XXXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (248.0, 8.0, 60.0, 50.0)),
            ("f", (298.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn vertical_flex_container_sizes_by_content_when_auto() {
    // An auto-sized vertical flex container takes its width from the lines of
    // its items and its height from their inline sizes, whatever is definite.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl"><div id=a>XXXXX</div><div id=b>XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 10.0, 80.0)),
            ("a", (8.0, 8.0, 10.0, 50.0)),
            ("b", (8.0, 58.0, 10.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;height:200px"><div id=a>XXXXX</div><div id=b>XXX XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 10.0, 200.0)),
            ("a", (8.0, 8.0, 10.0, 50.0)),
            ("b", (8.0, 58.0, 10.0, 70.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px"><div id=a>XXXXX</div><div id=b>XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 80.0)),
            ("a", (8.0, 8.0, 300.0, 50.0)),
            ("b", (8.0, 58.0, 300.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:200px"><div id=a style="width:30px">XXXXX</div><div id=b style="width:20px">XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 50.0, 200.0)),
            ("a", (28.0, 8.0, 30.0, 200.0)),
            ("b", (8.0, 8.0, 20.0, 200.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:200px"><div id=a>XXXXX</div><div id=b>XXX XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 20.0, 200.0)),
            ("a", (18.0, 8.0, 10.0, 200.0)),
            ("b", (8.0, 8.0, 10.0, 200.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl"><div id=a>XXXXX</div><div id=b>XXX XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 20.0, 70.0)),
            ("a", (18.0, 8.0, 10.0, 70.0)),
            ("b", (8.0, 8.0, 10.0, 70.0)),
        ],
    );
    check(
        r#"<div style="width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;height:fit-content"><div id=a>XXXXX</div><div id=b>XXX</div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 10.0, 80.0)),
            ("a", (8.0, 8.0, 10.0, 50.0)),
            ("b", (8.0, 58.0, 10.0, 30.0)),
        ],
    );
    check(
        r#"<div style="width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;height:max-content;width:100px"><div id=a>XXXXX</div><div id=b>XXX</div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 100.0, 80.0)),
            ("a", (8.0, 8.0, 100.0, 50.0)),
            ("b", (8.0, 58.0, 100.0, 30.0)),
        ],
    );
}

#[test]
fn vertical_flex_items_take_percentages_of_their_own_axes() {
    // A percentage size or `flex-basis` resolves against the container size on
    // the axis of the item size, not against the physical width.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:50%;height:25%"></div><div id=b style="flex:0 0 30%;width:20px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (158.0, 8.0, 150.0, 50.0)),
            ("b", (288.0, 58.0, 20.0, 60.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="writing-mode:horizontal-tb;width:50%;height:50%"><div style="width:10px;height:10px"></div></div><div id=b style="writing-mode:horizontal-tb;width:20px">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (158.0, 8.0, 150.0, 100.0)),
            ("b", (288.0, 108.0, 20.0, 10.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="height:50%;width:30px"></div><div id=b style="height:75%;width:20px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 8.0, 30.0, 80.0)),
            ("b", (288.0, 88.0, 20.0, 120.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="width:60px;height:100px"><div id=x style="width:50%;height:50%"></div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (248.0, 8.0, 60.0, 100.0)),
            ("x", (278.0, 8.0, 30.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="flex:1 1 content">XXXXX</div><div id=b style="flex:0 0 40%">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 120.0)),
            ("b", (8.0, 128.0, 300.0, 80.0)),
        ],
    );
}

#[test]
fn vertical_flex_items_clamp_between_min_and_max() {
    // Item minimums and maximums act on the inline axis in `min-height` and
    // `max-height` and on the block axis in `min-width` and `max-width`; the
    // automatic minimum size follows the content.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="flex:1;min-height:60px;max-height:90px;width:30px"></div><div id=b style="flex:1;min-width:50px;height:20px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (278.0, 8.0, 30.0, 90.0)),
            ("b", (8.0, 98.0, 300.0, 110.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:50px;height:100px"><div id=a style="width:80px;flex:0 1 20px"><div style="width:60px;height:5px"></div></div><div id=b style="width:30px;flex:0 1 0px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 50.0, 100.0)),
            ("a", (-2.0, 8.0, 60.0, 100.0)),
            ("b", (-2.0, 8.0, 0.0, 100.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:100px;height:50px"><div id=a style="height:80px;flex:0 1 20px;width:10px"><div style="width:5px;height:60px"></div></div><div id=b style="height:30px;flex:0 1 0px;width:10px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 100.0, 50.0)),
            ("a", (98.0, 8.0, 10.0, 60.0)),
            ("b", (98.0, 68.0, 10.0, 0.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="height:60px;aspect-ratio:2/1"></div><div id=b style="width:30px;aspect-ratio:1/3"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 60.0)),
            ("b", (278.0, 68.0, 30.0, 90.0)),
        ],
    );
}

#[test]
fn vertical_flex_items_in_a_horizontal_container_keep_their_own_axes() {
    // A vertical item of a horizontal flex container measures itself on its
    // own axes, stretched or not, in a row or a column.
    check(
        r#"<div id=c style="display:flex;width:300px;height:200px"><div id=a style="writing-mode:vertical-rl"><div style="width:30px;height:20px"></div></div><div id=b style="writing-mode:vertical-rl;width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 30.0, 200.0)),
            ("b", (38.0, 8.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;width:300px;height:200px"><div id=a style="writing-mode:vertical-rl">XXXXX</div><div id=b style="writing-mode:vertical-rl;height:60px">XXX XXX XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 10.0, 200.0)),
            ("b", (18.0, 8.0, 30.0, 60.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;width:300px;height:200px"><div id=a style="writing-mode:vertical-rl">XXXXX</div><div id=b style="writing-mode:vertical-rl;height:60px">XXX XXX XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 50.0)),
            ("b", (8.0, 58.0, 300.0, 60.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;align-items:stretch;width:300px;height:200px"><div id=a style="writing-mode:vertical-rl;height:auto">XXXXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 10.0, 200.0)),
        ],
    );
}

#[test]
fn horizontal_items_of_a_vertical_flex_container_are_orthogonal() {
    // A horizontal item of a vertical flex container has its width and height
    // read on the physical axes and gets the container axes only as available
    // space and stretch.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="writing-mode:horizontal-tb"><div style="width:30px;height:20px"></div></div><div id=b style="writing-mode:horizontal-tb;width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 20.0)),
            ("b", (268.0, 28.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="writing-mode:horizontal-tb">XXXXX</div><div id=b style="writing-mode:horizontal-tb;width:50px">XX XX XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 10.0)),
            ("b", (258.0, 18.0, 50.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="writing-mode:horizontal-tb">XXXXX</div><div id=b style="writing-mode:horizontal-tb;width:50px">XX XX XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (258.0, 8.0, 50.0, 200.0)),
            ("b", (208.0, 8.0, 50.0, 200.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="writing-mode:horizontal-tb;width:max-content;height:20px">XXXXX</div><div id=b style="writing-mode:horizontal-tb;width:min-content;height:30px">XX XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (258.0, 8.0, 50.0, 20.0)),
            ("b", (288.0, 28.0, 20.0, 30.0)),
        ],
    );
}

#[test]
fn nested_vertical_flex_containers_in_horizontal_ones() {
    // A vertical flex container as an item of a horizontal one, and the other
    // way round, lays out on its own axes; its parent uses its physical size.
    check(
        r#"<div id=c style="display:flex;width:300px;height:200px"><div id=a style="display:flex;writing-mode:vertical-rl;width:80px;height:100px"><div id=a1 style="width:20px;height:30px"></div><div id=a2 style="width:10px;height:40px"></div></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 80.0, 100.0)),
            ("a1", (68.0, 8.0, 20.0, 30.0)),
            ("a2", (78.0, 38.0, 10.0, 40.0)),
            ("b", (88.0, 8.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="display:flex;writing-mode:horizontal-tb;width:80px;height:100px"><div id=a1 style="width:20px;height:30px"></div><div id=a2 style="width:10px;height:40px"></div></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (228.0, 8.0, 80.0, 100.0)),
            ("a1", (228.0, 8.0, 20.0, 30.0)),
            ("a2", (248.0, 8.0, 10.0, 40.0)),
            ("b", (268.0, 108.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=p style="width:300px;height:200px"><div id=c style="display:flex;writing-mode:vertical-rl;width:80px;height:100px"><div id=a style="width:20px;height:30px"></div><div id=b style="width:10px;height:40px"></div></div></div>"#,
        &[
            ("p", (8.0, 8.0, 300.0, 200.0)),
            ("c", (8.0, 8.0, 80.0, 100.0)),
            ("a", (68.0, 8.0, 20.0, 30.0)),
            ("b", (78.0, 38.0, 10.0, 40.0)),
        ],
    );
    check(
        r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="display:flex;width:80px;height:100px"><div id=a style="width:20px;height:30px"></div><div id=b style="width:10px;height:40px"></div></div></div>"#,
        &[
            ("p", (8.0, 8.0, 300.0, 200.0)),
            ("c", (228.0, 8.0, 80.0, 100.0)),
            ("a", (288.0, 8.0, 20.0, 30.0)),
            ("b", (298.0, 38.0, 10.0, 40.0)),
        ],
    );
}

#[test]
fn nested_vertical_flex_containers_in_vertical_ones() {
    // Nested vertical flex containers, in the same or the other vertical
    // writing mode, direction or axis.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="display:flex;width:80px;height:100px;flex-direction:column"><div id=a1 style="width:20px;height:30px"></div><div id=a2 style="width:10px;height:40px"></div></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (228.0, 8.0, 80.0, 100.0)),
            ("a1", (288.0, 8.0, 20.0, 30.0)),
            ("a2", (278.0, 8.0, 10.0, 40.0)),
            ("b", (268.0, 108.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="display:flex"><div id=a1 style="width:20px;height:30px"></div><div id=a2 style="width:10px;height:40px"></div></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 70.0)),
            ("a1", (288.0, 8.0, 20.0, 30.0)),
            ("a2", (298.0, 38.0, 10.0, 40.0)),
            ("b", (268.0, 78.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="display:flex;writing-mode:vertical-lr;width:100px"><div id=a1 style="width:20px;height:30px"></div><div id=a2 style="width:10px;height:40px"></div></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (208.0, 8.0, 100.0, 70.0)),
            ("a1", (208.0, 8.0, 20.0, 30.0)),
            ("a2", (208.0, 38.0, 10.0, 40.0)),
            ("b", (268.0, 78.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;direction:rtl;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="display:flex;direction:ltr"><div id=a1 style="width:20px;height:30px"></div><div id=a2 style="width:10px;height:40px"></div></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 138.0, 300.0, 70.0)),
            ("a1", (288.0, 138.0, 20.0, 30.0)),
            ("a2", (298.0, 168.0, 10.0, 40.0)),
            ("b", (268.0, 88.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;width:300px;height:200px"><div id=a style="display:flex;writing-mode:vertical-rl;flex-direction:column"><div id=a1 style="display:flex;writing-mode:horizontal-tb"><div id=a2 style="width:20px;height:30px"></div></div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 20.0, 200.0)),
            ("a1", (8.0, 8.0, 20.0, 200.0)),
            ("a2", (8.0, 8.0, 20.0, 30.0)),
        ],
    );
}

#[test]
fn nested_auto_sized_flex_containers_follow_their_writing_modes() {
    // Auto-sized flex containers nested in vertical, horizontal and vertical
    // writing modes size by their contents.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="display:flex"><div id=a1>XXX</div><div id=a2>XX XX</div></div><div id=b>XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 80.0)),
            ("a1", (8.0, 8.0, 300.0, 30.0)),
            ("a2", (8.0, 38.0, 300.0, 50.0)),
            ("b", (8.0, 88.0, 300.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="display:flex;writing-mode:horizontal-tb"><div id=a1>XXX</div><div id=a2>XX XX</div></div><div id=b>XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 10.0)),
            ("a1", (8.0, 8.0, 30.0, 10.0)),
            ("a2", (38.0, 8.0, 50.0, 10.0)),
            ("b", (8.0, 18.0, 300.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;width:300px;height:200px"><div id=a style="display:flex;writing-mode:vertical-rl"><div id=a1>XXX</div><div id=a2>XX XX</div></div><div id=b>XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 10.0, 200.0)),
            ("a1", (8.0, 8.0, 10.0, 30.0)),
            ("a2", (8.0, 38.0, 10.0, 50.0)),
            ("b", (18.0, 8.0, 20.0, 200.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;width:200px"><div id=a style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:100px;"><div id=a1 style="width:20px">XX</div></div><div id=b>XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 20.0, 100.0)),
            ("a1", (8.0, 8.0, 20.0, 100.0)),
            ("b", (28.0, 8.0, 30.0, 100.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;height:150px"><div id=a style="display:flex;writing-mode:vertical-rl"><div id=a1 style="width:20px">XX</div></div><div id=b>XXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 784.0, 150.0)),
            ("a", (8.0, 8.0, 20.0, 150.0)),
            ("a1", (8.0, 8.0, 20.0, 20.0)),
            ("b", (28.0, 8.0, 30.0, 150.0)),
        ],
    );
}

#[test]
fn vertical_flex_container_as_an_inline_level_box() {
    // An `inline-flex` container in a vertical writing mode takes its size
    // from its items like any other atomic inline.
    check(
        r#"<div style="width:400px;height:50px"><div id=c style="display:inline-flex;writing-mode:vertical-rl"><div id=a>XXX</div><div id=b>XXXXX</div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 10.0, 80.0)),
            ("a", (8.0, 8.0, 10.0, 30.0)),
            ("b", (8.0, 38.0, 10.0, 50.0)),
        ],
    );
    check(
        r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="display:inline-flex;writing-mode:vertical-rl"><div id=a>XXX</div><div id=b>XXXXX</div></div></div>"#,
        &[
            ("p", (8.0, 8.0, 300.0, 200.0)),
            ("c", (298.0, 8.0, 10.0, 80.0)),
            ("a", (298.0, 8.0, 10.0, 30.0)),
            ("b", (298.0, 38.0, 10.0, 50.0)),
        ],
    );
}

#[test]
fn vertical_flex_items_that_scroll_or_overflow() {
    // Overflow does not change how items are placed on the axes of a vertical
    // container, and a scroll container item keeps its own size.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:100px;overflow:hidden"><div id=a style="width:30px;height:80px;flex:none"></div><div id=b style="width:40px;height:50px;flex:none"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (278.0, 8.0, 30.0, 80.0)),
            ("b", (268.0, 88.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:100px;overflow:auto"><div id=a style="width:30px;height:80px;flex:none"></div><div id=b style="width:40px;height:50px;flex:none"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (278.0, 8.0, 30.0, 80.0)),
            ("b", (268.0, 88.0, 40.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:100px"><div id=a style="overflow:hidden;flex:1 1 0;width:30px"><div style="width:30px;height:300px"></div></div><div id=b style="width:40px;height:50px"></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 100.0)),
            ("a", (278.0, 8.0, 30.0, 50.0)),
            ("b", (268.0, 58.0, 40.0, 50.0)),
        ],
    );
}

#[test]
fn vertical_flex_percentages_follow_the_container_height() {
    // A column container in a vertical writing mode lays its items out along
    // the block axis, so the 150px height is the inline size their percentage
    // edges refer to; an item that stretches across the container fills it.
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:150px;align-items:flex-start"><div id=i style="padding-bottom:10%">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 10.0, 150.0)),
            ("i", (8.0, 8.0, 10.0, 35.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:150px"><div id=i style="flex-basis:0;padding-bottom:10%">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 10.0, 150.0)),
            ("i", (8.0, 8.0, 10.0, 150.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;height:150px"><div id=i style="padding-bottom:10%;padding-right:10%">XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 25.0, 150.0)),
            ("i", (8.0, 8.0, 25.0, 35.0)),
        ],
    );
}

#[test]
fn item_keyword_sizes_on_the_cross_axis_do_not_stretch() {
    // A keyword inline size on an item of a vertical row container is a main
    // size, so the item still stretches across the block axis, and a keyword
    // inline size on an item of a vertical column container is a cross size,
    // so it does not stretch.
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="height:max-content">XXXXX</div><div id=b style="height:fit-content;width:20px">XXX</div><div id=d style="height:min-content;width:20px">XX XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 50.0)),
            ("b", (288.0, 58.0, 20.0, 30.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="inline-size:max-content">XXXXX</div><div id=b style="inline-size:min-content;block-size:20px">XX XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 50.0)),
            ("b", (288.0, 58.0, 20.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="inline-size:max-content">XXXXX</div><div id=b style="inline-size:min-content;block-size:20px">XX XX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (298.0, 8.0, 10.0, 50.0)),
            ("b", (278.0, 8.0, 20.0, 20.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="height:max-content">XXXXX</div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 50.0)),
        ],
    );
    check(
        r#"<div id=c style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=a style="height:max-content"><div style="width:5px;height:50px"></div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 300.0, 200.0)),
            ("a", (8.0, 8.0, 300.0, 50.0)),
        ],
    );
}
