//! The height of a column flex container whose height is `auto` is the sum of
//! the hypothetical main sizes of its items, that is their flex base sizes
//! clamped by their `min-height` and `max-height` (CSS Flexbox 1 §9.9.1). An
//! item that cannot grow or shrink inside a container of that height therefore
//! keeps its flex basis, and a `max-height` smaller than the basis caps it. The
//! container's own `min-height` and `max-height` then flex the items inside
//! it. The expected boxes are the ones Chromium lays out for the same markup on
//! an 800x600 page.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::HashMap;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// Twelve characters: one 120px line in a horizontal writing mode, wrapped to
/// five lines in a 100px wide box.
const WORDS: &str = "xxxx xxxx xx xxxx xxxx xx xxxx xxxx xx";
/// A single 120px line of text.
const LINE: &str = "xxxx xxxx xx";
/// A line that fits the 800px page on its own.
const LONG: &str = "xxxx xxxx xx xxxx xxxx xx xxxx xxxx xx xxxx xxxx xx xxxx xxxx xx xxxx";

/// The border box `(x, y, width, height)` of every element with an `id` on the
/// first page of an 800x600 document that holds `body`.
fn boxes(body: &str) -> HashMap<String, [f32; 4]> {
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
    let mut found = HashMap::new();
    for fragment in page.fragments() {
        let Some(id) = dom.attr(fragment.node(), "id") else {
            continue;
        };
        let rect = fragment.rect();
        found
            .entry(id.to_owned())
            .or_insert([rect.x, rect.y, rect.width, rect.height]);
    }
    found
}

#[track_caller]
fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    let found = boxes(body);
    for (id, want) in expected {
        let got = found.get(*id).unwrap_or_else(|| panic!("no box `{id}`"));
        assert!(
            got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.01),
            "`{id}` is {got:?}, expected {want:?} for {body}"
        );
    }
}

/// A column flex container `id=c` with `style` that holds `items`.
fn column(style: &str, items: &str) -> String {
    format!("<div id=c style=\"display:flex;flex-direction:column;{style}\">{items}</div>")
}

/// A flex item `id` with `style` and `text`.
fn item(id: &str, style: &str, text: &str) -> String {
    format!("<div id={id} style=\"{style}\">{text}</div>")
}

/// A column container `id=c` of an 800px page with the one item `id=a` in a
/// vertical writing mode, so that its flex basis is a length along the
/// item's inline axis.
#[track_caller]
fn assert_vertical_item(item_style: &str, height: f32) {
    let body = column(
        "",
        &item("a", &format!("writing-mode:vertical-rl;{item_style}"), LINE),
    );
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 800.0, height]),
            ("a", [0.0, 0.0, 800.0, height]),
        ],
    );
}

/// A column container `id=c` of an 800px page with the one clipping horizontal
/// item `id=a`.
#[track_caller]
fn assert_horizontal_item(item_style: &str, height: f32) {
    let body = column(
        "",
        &item("a", &format!("overflow:hidden;{item_style}"), LONG),
    );
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 800.0, height]),
            ("a", [0.0, 0.0, 800.0, height]),
        ],
    );
}

/// A column container `id=c` of width 100px with the one item `id=a`.
#[track_caller]
fn assert_narrow_item(item_style: &str, height: f32) {
    let body = column("width:100px", &item("a", item_style, WORDS));
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 100.0, height]),
            ("a", [0.0, 0.0, 100.0, height]),
        ],
    );
}

#[test]
fn a_rigid_vertical_item_keeps_its_flex_basis() {
    // Whatever the grow and shrink factors say, the container is as tall as
    // the basis: the item has no free space to use there.
    assert_vertical_item("flex-basis:60px", 60.0);
    assert_vertical_item("flex:0 1 60px", 60.0);
    assert_vertical_item("flex:1 0 60px", 60.0);
    assert_vertical_item("flex:1 1 60px", 60.0);
    assert_vertical_item("flex:0 0 60px", 60.0);
}

#[test]
fn an_automatic_basis_still_takes_the_content_size() {
    assert_vertical_item("flex:0 1 auto", 120.0);
    assert_vertical_item("flex:1 1 auto", 120.0);
}

#[test]
fn max_height_caps_the_basis_of_a_vertical_item() {
    assert_vertical_item("flex:0 0 300px;max-height:100px", 100.0);
    assert_vertical_item("flex:0 1 300px;max-height:100px", 100.0);
}

#[test]
fn min_height_raises_the_basis_of_a_vertical_item() {
    assert_vertical_item("flex:0 1 30px;min-height:100px", 100.0);
}

#[test]
fn max_height_caps_the_basis_of_a_horizontal_item() {
    assert_horizontal_item("flex:0 0 300px;max-height:100px", 100.0);
    assert_horizontal_item("flex:0 1 300px;max-height:100px", 100.0);
    assert_horizontal_item("flex:0 0 300px;max-height:100px;display:grid", 100.0);
    // Without a clip the item's content does not change the box.
    let visible = column("", &item("a", "flex:0 0 300px;max-height:100px", LONG));
    assert_boxes(
        &visible,
        &[
            ("c", [0.0, 0.0, 800.0, 100.0]),
            ("a", [0.0, 0.0, 800.0, 100.0]),
        ],
    );
}

#[test]
fn min_height_raises_the_basis_of_a_horizontal_item() {
    assert_horizontal_item("flex:0 0 300px;min-height:400px", 400.0);
}

#[test]
fn a_height_does_not_replace_the_flex_basis() {
    // `flex-basis` wins over `height`, so the container follows the basis.
    assert_horizontal_item("flex:0 0 300px;height:100px", 300.0);
    assert_narrow_item("flex:0 1 20px;height:70px;min-height:0", 20.0);
    // With an automatic basis the `height` is the basis.
    assert_horizontal_item("height:100px", 100.0);
    assert_narrow_item("height:70px;flex:0 1 auto", 70.0);
}

#[test]
fn a_growing_or_shrinking_horizontal_item_keeps_its_basis() {
    assert_horizontal_item("flex:1 0 20px", 20.0);
    assert_horizontal_item("flex:0 1 60px", 60.0);
}

#[test]
fn a_container_with_a_height_is_filled_by_a_growing_item() {
    let body = column(
        "height:200px",
        &item("a", "flex:1 1 20px;overflow:hidden", LONG),
    );
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 800.0, 200.0]),
            ("a", [0.0, 0.0, 800.0, 200.0]),
        ],
    );
}

#[test]
fn two_capped_items_add_up() {
    let body = column(
        "",
        &(item("a", "flex:0 0 300px;max-height:100px;overflow:hidden", LONG)
            + &item("b", "flex:0 0 200px;max-height:50px;overflow:hidden", LONG)),
    );
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 800.0, 150.0]),
            ("a", [0.0, 0.0, 800.0, 100.0]),
            ("b", [0.0, 100.0, 800.0, 50.0]),
        ],
    );
}

#[test]
fn a_zero_basis_item_with_a_zero_minimum_collapses() {
    assert_narrow_item("flex:1 1 0px;min-height:0", 0.0);
    let two = column(
        "width:100px",
        &(item("a", "flex:1 1 0px;min-height:0", WORDS)
            + &item("b", "flex:2 1 0px;min-height:0", WORDS)),
    );
    assert_boxes(
        &two,
        &[
            ("c", [0.0, 0.0, 100.0, 0.0]),
            ("a", [0.0, 0.0, 100.0, 0.0]),
            ("b", [0.0, 0.0, 100.0, 0.0]),
        ],
    );
}

#[test]
fn a_minimum_of_zero_lets_a_basis_below_the_content_stand() {
    assert_narrow_item("flex:0 1 20px;min-height:0", 20.0);
    assert_narrow_item("flex:0 0 20px;min-height:0", 20.0);
    assert_narrow_item("flex:1 0 20px;min-height:0", 20.0);
}

#[test]
fn an_automatic_minimum_keeps_the_content_size() {
    assert_narrow_item("flex:1 1 0px", 50.0);
    assert_narrow_item("flex:0 1 20px", 50.0);
    // A percentage of the open height is not a length: the content gives it.
    assert_narrow_item("flex:1 1 0%;min-height:0", 50.0);
    assert_narrow_item("flex:0 1 50%;min-height:0", 50.0);
}

#[test]
fn one_item_with_an_automatic_minimum_next_to_a_collapsed_one() {
    let body = column(
        "width:100px",
        &(item("a", "flex:1 1 0px;min-height:0", WORDS) + &item("b", "flex:1 1 0px", WORDS)),
    );
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 100.0, 50.0]),
            ("a", [0.0, 0.0, 100.0, 0.0]),
            ("b", [0.0, 0.0, 100.0, 50.0]),
        ],
    );
}

#[test]
fn a_calc_basis_is_a_length() {
    assert_narrow_item("flex:0 1 calc(10px + 15px);min-height:0", 25.0);
}

#[test]
fn a_scroll_container_item_keeps_its_basis() {
    assert_narrow_item("flex:0 1 20px;overflow:auto", 20.0);
}

#[test]
fn the_container_minimum_height_grows_the_items() {
    let grow = column(
        "width:100px;min-height:200px",
        &(item("a", "flex:1 1 20px;min-height:0", WORDS)
            + &item("b", "flex:2 1 20px;min-height:0", WORDS)),
    );
    assert_boxes(
        &grow,
        &[
            ("c", [0.0, 0.0, 100.0, 200.0]),
            ("a", [0.0, 0.0, 100.0, 73.33]),
            ("b", [0.0, 73.33, 100.0, 126.67]),
        ],
    );
    // An item that cannot grow stays at its basis.
    let rigid = column(
        "width:100px;min-height:200px",
        &item("a", "flex:0 1 20px;min-height:0", WORDS),
    );
    assert_boxes(
        &rigid,
        &[
            ("c", [0.0, 0.0, 100.0, 200.0]),
            ("a", [0.0, 0.0, 100.0, 20.0]),
        ],
    );
    // A minimum below the items' height changes nothing.
    let small = column(
        "width:100px;min-height:10px",
        &item("a", "flex:1 1 40px;min-height:0", WORDS),
    );
    assert_boxes(
        &small,
        &[
            ("c", [0.0, 0.0, 100.0, 40.0]),
            ("a", [0.0, 0.0, 100.0, 40.0]),
        ],
    );
}

#[test]
fn the_container_maximum_height_shrinks_the_items() {
    let shrink = column(
        "width:100px;max-height:30px",
        &(item("a", "flex:0 1 100px;min-height:0", WORDS)
            + &item("b", "flex:0 1 100px;min-height:0", WORDS)),
    );
    assert_boxes(
        &shrink,
        &[
            ("c", [0.0, 0.0, 100.0, 30.0]),
            ("a", [0.0, 0.0, 100.0, 15.0]),
            ("b", [0.0, 15.0, 100.0, 15.0]),
        ],
    );
    // A maximum above the items' height changes nothing.
    let big = column(
        "width:100px;max-height:500px",
        &(item("a", "flex:1 1 40px;min-height:0", WORDS)
            + &item("b", "flex:1 1 40px;min-height:0", WORDS)),
    );
    assert_boxes(
        &big,
        &[
            ("c", [0.0, 0.0, 100.0, 80.0]),
            ("a", [0.0, 0.0, 100.0, 40.0]),
            ("b", [0.0, 40.0, 100.0, 40.0]),
        ],
    );
}

#[test]
fn gaps_padding_border_and_margins_add_to_the_basis() {
    let gap = column(
        "width:100px;gap:7px",
        &(item("a", "flex:0 1 20px;min-height:0", WORDS)
            + &item("b", "flex:0 1 30px;min-height:0", WORDS)),
    );
    assert_boxes(
        &gap,
        &[
            ("c", [0.0, 0.0, 100.0, 57.0]),
            ("a", [0.0, 0.0, 100.0, 20.0]),
            ("b", [0.0, 27.0, 100.0, 30.0]),
        ],
    );
    let frame = column(
        "width:100px;padding:5px;border:3px solid",
        &item("a", "flex:0 1 20px;min-height:0", WORDS),
    );
    assert_boxes(
        &frame,
        &[
            ("c", [0.0, 0.0, 100.0, 36.0]),
            ("a", [8.0, 8.0, 84.0, 20.0]),
        ],
    );
    let margins = column(
        "width:100px",
        &item("a", "flex:0 1 20px;min-height:0;margin:4px 0 6px", WORDS),
    );
    assert_boxes(
        &margins,
        &[
            ("c", [0.0, 0.0, 100.0, 30.0]),
            ("a", [0.0, 4.0, 100.0, 20.0]),
        ],
    );
}

#[test]
fn a_content_box_item_adds_its_padding_and_border_to_the_basis() {
    assert_narrow_item(
        "flex:0 1 20px;min-height:0;box-sizing:content-box;padding:5px;border:2px solid",
        34.0,
    );
}

#[test]
fn rigid_and_automatic_items_in_one_container() {
    let mixed = column(
        "width:100px",
        &(item("a", "flex:0 1 20px;min-height:0", WORDS) + &item("b", "flex:0 1 auto", "yy")),
    );
    assert_boxes(
        &mixed,
        &[
            ("c", [0.0, 0.0, 100.0, 30.0]),
            ("a", [0.0, 0.0, 100.0, 20.0]),
            ("b", [0.0, 20.0, 100.0, 10.0]),
        ],
    );
    // An automatic basis is capped by `max-height` as well.
    assert_narrow_item("flex:0 1 auto;max-height:15px", 15.0);
    let capped = column(
        "width:100px",
        &(item("a", "flex:0 1 80px;max-height:15px;min-height:0", WORDS) + &item("b", "", "yy")),
    );
    assert_boxes(
        &capped,
        &[
            ("c", [0.0, 0.0, 100.0, 25.0]),
            ("a", [0.0, 0.0, 100.0, 15.0]),
            ("b", [0.0, 15.0, 100.0, 10.0]),
        ],
    );
}

#[test]
fn a_reversed_column_places_the_items_from_the_bottom() {
    let body = column(
        "width:100px;flex-direction:column-reverse",
        &(item("a", "flex:0 1 20px;min-height:0", WORDS)
            + &item("b", "flex:0 1 30px;min-height:0", WORDS)),
    );
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 100.0, 50.0]),
            ("a", [0.0, 30.0, 100.0, 20.0]),
            ("b", [0.0, 0.0, 100.0, 30.0]),
        ],
    );
}

#[test]
fn cross_axis_alignment_does_not_change_the_height() {
    let body = column(
        "width:100px;align-items:center",
        &item("a", "flex:0 1 20px;min-height:0;width:50px", WORDS),
    );
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 100.0, 20.0]),
            ("a", [25.0, 0.0, 50.0, 20.0]),
        ],
    );
}

#[test]
fn a_nested_column_container_is_a_rigid_item_too() {
    let body = column(
        "width:100px",
        &item(
            "a",
            "display:flex;flex-direction:column;flex:0 1 20px;min-height:0",
            &item("d", "flex:0 1 50px;min-height:0", WORDS),
        ),
    );
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 100.0, 20.0]),
            ("a", [0.0, 0.0, 100.0, 20.0]),
            ("d", [0.0, 0.0, 100.0, 20.0]),
        ],
    );
}

#[test]
fn a_column_container_in_a_row_container() {
    let short = format!(
        "<div style=\"display:flex;width:300px\">{}{}</div>",
        column(
            "width:100px",
            &item("a", "flex:0 1 20px;min-height:0", WORDS)
        ),
        item("s", "width:50px", "yy"),
    );
    assert_boxes(
        &short,
        &[
            ("c", [0.0, 0.0, 100.0, 20.0]),
            ("a", [0.0, 0.0, 100.0, 20.0]),
            ("s", [100.0, 0.0, 50.0, 20.0]),
        ],
    );
    // A taller sibling stretches the container, and the growing item fills it.
    let tall = format!(
        "<div style=\"display:flex;width:300px\">{}{}</div>",
        column(
            "width:100px",
            &item("a", "flex:1 1 20px;min-height:0", WORDS)
        ),
        item("s", "width:50px;height:150px", "yy"),
    );
    assert_boxes(
        &tall,
        &[
            ("c", [0.0, 0.0, 100.0, 150.0]),
            ("a", [0.0, 0.0, 100.0, 150.0]),
            ("s", [100.0, 0.0, 50.0, 150.0]),
        ],
    );
}

#[test]
fn a_column_container_in_a_grid_row() {
    let container = column("", &item("a", "flex:1 1 20px;min-height:0", WORDS));
    let body = format!(
        "<div style=\"display:grid;width:300px;grid-template-columns:100px 100px\">{container}{}</div>",
        item("s", "height:120px", "yy"),
    );
    assert_boxes(
        &body,
        &[
            ("c", [0.0, 0.0, 100.0, 120.0]),
            ("a", [0.0, 0.0, 100.0, 120.0]),
            ("s", [100.0, 0.0, 100.0, 120.0]),
        ],
    );
}
