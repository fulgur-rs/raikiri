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

/// The height of the spacer that `below_spacer` puts above a container.
const SPACER: f32 = 300.0;

/// `body` below a spacer, so that boxes a negative margin pulls up stay on the
/// page.
fn below_spacer(body: &str) -> String {
    format!("<div style=\"height:{SPACER}px\"></div>{body}")
}

#[test]
fn negative_margins_leave_free_space_that_a_growing_item_fills() {
    // The sum of the outer sizes is below zero, which the container's height
    // cannot be, so the item has the rest of the space to grow into.
    let grow = |style: &str| below_spacer(&column("width:100px", &item("a", style, "")));
    assert_boxes(
        &grow("flex:1 1 100px;min-height:0;margin-top:-200px"),
        &[
            ("c", [0.0, SPACER, 100.0, 0.0]),
            ("a", [0.0, SPACER - 200.0, 100.0, 200.0]),
        ],
    );
    assert_boxes(
        &grow("flex:1 1 100px;min-height:0;margin-bottom:-150px"),
        &[
            ("c", [0.0, SPACER, 100.0, 0.0]),
            ("a", [0.0, SPACER, 100.0, 150.0]),
        ],
    );
    // A percentage refers to the width of the container.
    assert_boxes(
        &grow("flex:1 1 100px;min-height:0;margin-top:-300%"),
        &[
            ("c", [0.0, SPACER, 100.0, 0.0]),
            ("a", [0.0, SPACER - 300.0, 100.0, 300.0]),
        ],
    );
    // An item that cannot grow keeps its basis.
    assert_boxes(
        &grow("flex:0 1 100px;min-height:0;margin-top:-200px"),
        &[
            ("c", [0.0, SPACER, 100.0, 0.0]),
            ("a", [0.0, SPACER - 200.0, 100.0, 100.0]),
        ],
    );
    // A margin that leaves the sum above zero changes nothing.
    assert_boxes(
        &grow("flex:1 1 100px;min-height:0;margin-top:-30px"),
        &[
            ("c", [0.0, SPACER, 100.0, 70.0]),
            ("a", [0.0, SPACER - 30.0, 100.0, 100.0]),
        ],
    );
}

#[test]
fn negative_margins_share_the_free_space_by_flex_grow() {
    let body = below_spacer(&column(
        "width:100px",
        &(item("a", "flex:1 1 100px;min-height:0;margin-top:-250px", "")
            + &item("b", "flex:2 1 20px;min-height:0", "")),
    ));
    assert_boxes(
        &body,
        &[
            ("c", [0.0, SPACER, 100.0, 0.0]),
            ("a", [0.0, SPACER - 250.0, 100.0, 143.33]),
            ("b", [0.0, SPACER - 106.67, 100.0, 106.67]),
        ],
    );
    // Padding stays outside the free space.
    let padded = below_spacer(&column(
        "width:100px;padding:5px",
        &item("a", "flex:1 1 100px;min-height:0;margin-top:-200px", ""),
    ));
    assert_boxes(
        &padded,
        &[
            ("c", [0.0, SPACER, 100.0, 10.0]),
            ("a", [5.0, SPACER - 195.0, 90.0, 200.0]),
        ],
    );
    // An item of automatic size next to a growing one.
    let mixed = below_spacer(&column(
        "width:100px",
        &(item("a", "margin-bottom:-80px", WORDS) + &item("b", "flex:1 1 20px;min-height:0", "")),
    ));
    assert_boxes(
        &mixed,
        &[
            ("c", [0.0, SPACER, 100.0, 0.0]),
            ("a", [0.0, SPACER, 100.0, 50.0]),
            ("b", [0.0, SPACER - 30.0, 100.0, 30.0]),
        ],
    );
}

/// A column container `id=c` with `style` and the items `items`, in a block
/// of width 100px whose height is `height` (`auto` when it states none).
fn in_block(height: &str, style: &str, items: &str) -> String {
    let height = if height == "auto" {
        String::new()
    } else {
        format!(";height:{height}")
    };
    format!(
        "<div style=\"width:100px{height}\">{}</div>",
        column(style, items)
    )
}

#[test]
fn a_percentage_maximum_height_shrinks_the_items() {
    // The percentage refers to the 200px of the containing block.
    let one = in_block(
        "200px",
        "max-height:50%",
        &item("a", "flex:0 1 200px;min-height:0", ""),
    );
    assert_boxes(
        &one,
        &[
            ("c", [0.0, 0.0, 100.0, 100.0]),
            ("a", [0.0, 0.0, 100.0, 100.0]),
        ],
    );
    let calc = in_block(
        "200px",
        "max-height:calc(50% - 10px)",
        &item("a", "flex:0 1 200px;min-height:0", ""),
    );
    assert_boxes(
        &calc,
        &[
            ("c", [0.0, 0.0, 100.0, 90.0]),
            ("a", [0.0, 0.0, 100.0, 90.0]),
        ],
    );
    let two = in_block(
        "200px",
        "max-height:50%",
        &(item("a", "flex:0 1 150px;min-height:0", "")
            + &item("b", "flex:0 1 50px;min-height:0", "")),
    );
    assert_boxes(
        &two,
        &[
            ("c", [0.0, 0.0, 100.0, 100.0]),
            ("a", [0.0, 0.0, 100.0, 75.0]),
            ("b", [0.0, 75.0, 100.0, 25.0]),
        ],
    );
    // A maximum above the items' height changes nothing.
    let big = in_block(
        "200px",
        "max-height:90%",
        &item("a", "flex:0 1 60px;min-height:0", ""),
    );
    assert_boxes(
        &big,
        &[
            ("c", [0.0, 0.0, 100.0, 60.0]),
            ("a", [0.0, 0.0, 100.0, 60.0]),
        ],
    );
}

#[test]
fn a_percentage_minimum_height_grows_the_items() {
    let grow = in_block(
        "200px",
        "min-height:50%",
        &item("a", "flex:1 1 20px;min-height:0", ""),
    );
    assert_boxes(
        &grow,
        &[
            ("c", [0.0, 0.0, 100.0, 100.0]),
            ("a", [0.0, 0.0, 100.0, 100.0]),
        ],
    );
    let rigid = in_block(
        "200px",
        "min-height:50%",
        &item("a", "flex:0 1 20px;min-height:0", ""),
    );
    assert_boxes(
        &rigid,
        &[
            ("c", [0.0, 0.0, 100.0, 100.0]),
            ("a", [0.0, 0.0, 100.0, 20.0]),
        ],
    );
}

#[test]
fn a_percentage_bound_has_no_effect_in_a_block_of_open_height() {
    let max = in_block(
        "auto",
        "max-height:50%",
        &item("a", "flex:0 1 200px;min-height:0", ""),
    );
    assert_boxes(
        &max,
        &[
            ("c", [0.0, 0.0, 100.0, 200.0]),
            ("a", [0.0, 0.0, 100.0, 200.0]),
        ],
    );
    let min = in_block(
        "auto",
        "min-height:50%",
        &item("a", "flex:1 1 20px;min-height:0", ""),
    );
    assert_boxes(
        &min,
        &[
            ("c", [0.0, 0.0, 100.0, 20.0]),
            ("a", [0.0, 0.0, 100.0, 20.0]),
        ],
    );
}

/// One case: the style of a block around the container, the container style,
/// each item as `(style, content)`, and the border boxes Chromium gives the
/// container `c` and its items `i0`, `i1`, ...
type Case<'a> = (
    &'a str,
    &'a str,
    &'a [(&'a str, &'a str)],
    &'a [(&'a str, [f32; 4])],
);

#[track_caller]
fn assert_cases(cases: &[Case<'_>]) {
    for (parent, container, items, expected) in cases {
        let items: String = items
            .iter()
            .enumerate()
            .map(|(i, (style, text))| item(&format!("i{i}"), style, text))
            .collect();
        let body =
            format!("<div style=\"{parent}\"><div id=c style=\"{container}\">{items}</div></div>");
        assert_boxes(&body, expected);
    }
}

#[test]
fn a_wrapping_column_is_as_tall_as_its_longest_line_of_flex_bases() {
    // The items grow, but the height of the container is the sum of their
    // flex base sizes on its longest line, with its lines broken at its
    // `max-height`. A growing item then fills that height in its line.
    assert_cases(&[
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;",
            &[
                ("writing-mode:vertical-rl;flex:1 0 40px", "xxxx xxxx xxxx"),
                ("writing-mode:vertical-rl;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:vertical-rl;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 10.0, 120.0]),
                ("i0", [0.0, 0.0, 30.0, 40.0]),
                ("i1", [0.0, 40.0, 30.0, 50.0]),
                ("i2", [0.0, 90.0, 30.0, 30.0]),
            ],
        ),
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;max-height:70px;",
            &[
                ("writing-mode:vertical-rl;flex:1 0 40px", "xxxx xxxx xxxx"),
                ("writing-mode:vertical-rl;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:vertical-rl;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 60.0, 50.0]),
                ("i0", [0.0, 0.0, 30.0, 50.0]),
                ("i1", [30.0, 0.0, 20.0, 50.0]),
                ("i2", [50.0, 0.0, 10.0, 30.0]),
            ],
        ),
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap-reverse;max-height:70px;",
            &[
                ("writing-mode:vertical-rl;flex:1 0 40px", "xxxx xxxx xxxx"),
                ("writing-mode:vertical-rl;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:vertical-rl;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 60.0, 50.0]),
                ("i0", [30.0, 0.0, 30.0, 50.0]),
                ("i1", [10.0, 0.0, 20.0, 50.0]),
                ("i2", [0.0, 0.0, 10.0, 30.0]),
            ],
        ),
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;min-height:100px;max-height:110px;",
            &[
                ("writing-mode:vertical-rl;flex:1 0 40px", "xxxx xxxx xxxx"),
                ("writing-mode:vertical-rl;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:vertical-rl;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 30.0, 100.0]),
                ("i0", [0.0, 0.0, 30.0, 45.0]),
                ("i1", [0.0, 45.0, 30.0, 55.0]),
                ("i2", [30.0, 0.0, 10.0, 30.0]),
            ],
        ),
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;min-height:60px;",
            &[
                ("writing-mode:vertical-rl;flex:1 0 40px", "xxxx"),
                ("writing-mode:vertical-rl;flex:2 0 30px", "xx xx"),
            ],
            &[
                ("c", [0.0, 0.0, 10.0, 70.0]),
                ("i0", [0.0, 0.0, 20.0, 40.0]),
                ("i1", [0.0, 40.0, 20.0, 30.0]),
            ],
        ),
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;",
            &[
                ("writing-mode:vertical-rl;flex:1 0 auto", "xx"),
                ("writing-mode:vertical-rl;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:vertical-rl;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 10.0, 100.0]),
                ("i0", [0.0, 0.0, 20.0, 20.0]),
                ("i1", [0.0, 20.0, 20.0, 50.0]),
                ("i2", [0.0, 70.0, 20.0, 30.0]),
            ],
        ),
        (
            "",
            "display:flex;width:max-content;flex-direction:column;flex-wrap:wrap;max-height:45px;",
            &[
                ("writing-mode:vertical-rl;flex:1 0 40px", "xxxx"),
                ("writing-mode:vertical-rl;flex:2 0 30px", "xx xx"),
            ],
            &[
                ("c", [0.0, 0.0, 30.0, 40.0]),
                ("i0", [0.0, 0.0, 10.0, 40.0]),
                ("i1", [10.0, 0.0, 20.0, 40.0]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_column_is_as_wide_as_its_flex_bases() {
    // The main axis of a vertical column is horizontal: its width is the sum
    // of the flex base sizes of its items, on its longest line where it wraps
    // at its `max-width`.
    assert_cases(&[
        (
            "margin-top:100px",
            "display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:nowrap;",
            &[
                ("writing-mode:horizontal-tb;flex:1 0 40px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:horizontal-tb;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 100.0, 120.0, 10.0]),
                ("i0", [80.0, 100.0, 40.0, 10.0]),
                ("i1", [30.0, 100.0, 50.0, 10.0]),
                ("i2", [0.0, 100.0, 30.0, 10.0]),
            ],
        ),
        (
            "margin-top:100px",
            "display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:nowrap;min-width:60px;",
            &[
                ("writing-mode:horizontal-tb;flex:1 0 40px", "xxxx"),
                ("writing-mode:horizontal-tb;flex:2 0 30px", "xx xx"),
            ],
            &[
                ("c", [0.0, 100.0, 70.0, 10.0]),
                ("i0", [30.0, 100.0, 40.0, 10.0]),
                ("i1", [0.0, 100.0, 30.0, 10.0]),
            ],
        ),
        (
            "margin-top:100px",
            "display:flex;height:max-content;flex-direction:column;writing-mode:vertical-lr;flex-wrap:nowrap;min-width:100px;max-width:110px;",
            &[
                ("writing-mode:horizontal-tb;flex:1 0 auto", "xx"),
                ("writing-mode:horizontal-tb;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:horizontal-tb;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 100.0, 100.0, 10.0]),
                ("i0", [0.0, 100.0, 20.0, 10.0]),
                ("i1", [20.0, 100.0, 50.0, 10.0]),
                ("i2", [70.0, 100.0, 30.0, 10.0]),
            ],
        ),
        (
            "margin-top:100px",
            "display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;max-width:70px;",
            &[
                ("writing-mode:horizontal-tb;flex:1 0 40px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:horizontal-tb;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 100.0, 50.0, 60.0]),
                ("i0", [0.0, 100.0, 50.0, 30.0]),
                ("i1", [0.0, 130.0, 50.0, 20.0]),
                ("i2", [20.0, 150.0, 30.0, 10.0]),
            ],
        ),
        (
            "margin-top:100px",
            "display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap-reverse;min-width:100px;max-width:110px;",
            &[
                ("writing-mode:horizontal-tb;flex:1 0 40px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:horizontal-tb;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 100.0, 100.0, 30.0]),
                ("i0", [55.0, 100.0, 45.0, 30.0]),
                ("i1", [0.0, 100.0, 55.0, 30.0]),
                ("i2", [70.0, 90.0, 30.0, 10.0]),
            ],
        ),
        (
            "margin-top:100px",
            "display:inline-flex;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap;max-width:45px;",
            &[
                ("writing-mode:horizontal-tb;flex:1 0 40px", "xxxx"),
                ("writing-mode:horizontal-tb;flex:2 0 30px", "xx xx"),
            ],
            &[
                ("c", [0.0, 100.0, 40.0, 30.0]),
                ("i0", [0.0, 100.0, 40.0, 10.0]),
                ("i1", [0.0, 110.0, 40.0, 20.0]),
            ],
        ),
        (
            "margin-top:100px",
            "display:flex;height:max-content;flex-direction:column;writing-mode:vertical-lr;flex-wrap:wrap-reverse;max-width:70px;",
            &[
                ("writing-mode:horizontal-tb;flex:1 0 40px", "xxxx xxxx xxxx"),
                ("writing-mode:horizontal-tb;flex:1 0 50px", "xx xx xx xx"),
                ("writing-mode:horizontal-tb;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 100.0, 50.0, 60.0]),
                ("i0", [0.0, 130.0, 50.0, 30.0]),
                ("i1", [0.0, 110.0, 50.0, 20.0]),
                ("i2", [0.0, 100.0, 30.0, 10.0]),
            ],
        ),
    ]);
}

#[test]
fn a_wrapping_column_takes_bounded_bases_negative_margins_and_its_padding() {
    // A `max-height` below the flex basis bounds the item's share of its
    // line, a negative margin lowers it, and the padding and border of the
    // container leave less room for its lines than its `max-height`.
    assert_cases(&[
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;writing-mode:horizontal-tb;max-height:200px;",
            &[
                (
                    "writing-mode:vertical-rl;flex:1 0 120px;max-height:80px",
                    "xx xx",
                ),
                ("writing-mode:vertical-rl;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 10.0, 110.0]),
                ("i0", [0.0, 0.0, 10.0, 80.0]),
                ("i1", [0.0, 80.0, 10.0, 30.0]),
            ],
        ),
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;writing-mode:horizontal-tb;max-height:100px;",
            &[
                (
                    "writing-mode:vertical-rl;flex:1 0 120px;max-height:30px",
                    "xx",
                ),
                ("writing-mode:vertical-rl;flex:0 0 10px", "x"),
                ("writing-mode:vertical-rl;flex:1 0 50px", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 10.0, 90.0]),
                ("i0", [0.0, 0.0, 10.0, 30.0]),
                ("i1", [0.0, 30.0, 10.0, 10.0]),
                ("i2", [0.0, 40.0, 10.0, 50.0]),
            ],
        ),
        (
            "margin-top:100px",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;writing-mode:vertical-rl;max-width:200px;",
            &[
                (
                    "writing-mode:horizontal-tb;flex:1 0 120px;max-width:80px",
                    "xx xx",
                ),
                ("writing-mode:horizontal-tb;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 100.0, 110.0, 10.0]),
                ("i0", [30.0, 100.0, 80.0, 10.0]),
                ("i1", [0.0, 100.0, 30.0, 10.0]),
            ],
        ),
        (
            "margin-top:100px",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap-reverse;writing-mode:vertical-rl;min-width:60px;",
            &[
                (
                    "writing-mode:horizontal-tb;flex:1 0 120px;max-width:30px",
                    "xx",
                ),
                ("writing-mode:horizontal-tb;flex:0 0 10px", "x"),
                ("writing-mode:horizontal-tb;flex:1 0 50px", "xx"),
            ],
            &[
                ("c", [0.0, 100.0, 90.0, 10.0]),
                ("i0", [60.0, 100.0, 30.0, 10.0]),
                ("i1", [50.0, 100.0, 10.0, 10.0]),
                ("i2", [0.0, 100.0, 50.0, 10.0]),
            ],
        ),
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;writing-mode:horizontal-tb;min-height:60px;",
            &[
                ("flex:1 0 120px;max-height:80px;overflow:hidden", "xx"),
                ("flex:0 0 30px;overflow:hidden", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 20.0, 110.0]),
                ("i0", [0.0, 0.0, 20.0, 80.0]),
                ("i1", [0.0, 80.0, 20.0, 30.0]),
            ],
        ),
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;writing-mode:horizontal-tb;",
            &[
                ("flex:1 0 40px;margin-top:-50px;overflow:hidden", "xx"),
                ("flex:0 0 30px;overflow:hidden", "xx"),
            ],
            &[
                ("c", [0.0, 50.0, 20.0, 20.0]),
                ("i0", [0.0, 0.0, 20.0, 40.0]),
                ("i1", [0.0, 40.0, 20.0, 30.0]),
            ],
        ),
        (
            "",
            "display:inline-flex;flex-direction:column;flex-wrap:wrap;writing-mode:horizontal-tb;min-height:60px;",
            &[
                ("flex:1 0 40px;margin-top:-50px;overflow:hidden", "xx"),
                ("flex:0 0 30px;overflow:hidden", "xx"),
            ],
            &[
                ("c", [0.0, 50.0, 20.0, 60.0]),
                ("i0", [0.0, 0.0, 20.0, 80.0]),
                ("i1", [0.0, 80.0, 20.0, 30.0]),
            ],
        ),
        (
            "margin-top:100px",
            "display:inline-flex;flex-direction:column;flex-wrap:nowrap;writing-mode:vertical-rl;",
            &[
                ("flex:1 0 40px;margin-right:-50px;overflow:hidden", "xx"),
                ("flex:0 0 30px;overflow:hidden", "xx"),
            ],
            &[
                ("c", [0.0, 100.0, 20.0, 20.0]),
                ("i0", [30.0, 100.0, 40.0, 20.0]),
                ("i1", [0.0, 100.0, 30.0, 20.0]),
            ],
        ),
        (
            "",
            "display:flex;flex-direction:column;flex-wrap:wrap;writing-mode:horizontal-tb;box-sizing:border-box;padding:10px 7px;border:3px solid;max-height:100px;",
            &[
                ("writing-mode:vertical-rl;flex:1 0 40px", "xx"),
                ("writing-mode:vertical-rl;flex:1 0 50px", "xx"),
                ("writing-mode:vertical-rl;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("i0", [10.0, 13.0, 260.0, 50.0]),
                ("i1", [270.0, 13.0, 260.0, 50.0]),
                ("i2", [530.0, 13.0, 260.0, 30.0]),
            ],
        ),
        (
            "",
            "display:flex;flex-direction:column;flex-wrap:wrap;writing-mode:horizontal-tb;box-sizing:content-box;padding:10px 7px;border:3px solid;max-height:100px;",
            &[
                ("writing-mode:vertical-rl;flex:0 0 40px", "xx"),
                ("writing-mode:vertical-rl;flex:0 0 50px", "xx"),
                ("writing-mode:vertical-rl;flex:0 0 30px", "xx"),
            ],
            &[
                ("c", [0.0, 0.0, 800.0, 116.0]),
                ("i0", [10.0, 13.0, 390.0, 40.0]),
                ("i1", [10.0, 53.0, 390.0, 50.0]),
                ("i2", [400.0, 13.0, 390.0, 30.0]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_wrapping_column_resolves_its_padding_against_the_parent_inline_size() {
    // Percentage padding refers to the inline size of the containing block:
    // the height of a vertical parent and the width of a horizontal one. The
    // content-box maximum the container gets back from its found width
    // removes the same padding.
    let container = "display:inline-flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;box-sizing:content-box;max-width:100px;padding:0 10%;";
    let items: &[(&str, &str)] = &[
        ("writing-mode:horizontal-tb;flex:1 0 40px", "xxxx xxxx xxxx"),
        ("writing-mode:horizontal-tb;flex:1 0 50px", "xx xx xx xx"),
        ("writing-mode:horizontal-tb;flex:0 0 30px", "xx"),
    ];
    // A 300x200 parent that runs vertically: 20px of padding on each side.
    let in_vertical_parent: &[(&str, [f32; 4])] = &[
        ("c", [170.0, 100.0, 130.0, 30.0]),
        ("i0", [240.0, 100.0, 40.0, 30.0]),
        ("i1", [190.0, 100.0, 50.0, 30.0]),
        ("i2", [250.0, 130.0, 30.0, 10.0]),
    ];
    assert_cases(&[
        (
            "writing-mode:vertical-rl;width:300px;height:200px;margin-top:100px",
            container,
            items,
            in_vertical_parent,
        ),
        (
            "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px;margin-top:100px",
            container,
            items,
            in_vertical_parent,
        ),
        (
            "writing-mode:vertical-rl;display:grid;align-items:start;justify-items:start;width:300px;height:200px;margin-top:100px",
            container,
            items,
            in_vertical_parent,
        ),
        (
            "writing-mode:vertical-rl;width:200px;height:300px;margin-top:100px",
            container,
            items,
            &[
                ("c", [50.0, 100.0, 150.0, 30.0]),
                ("i0", [130.0, 100.0, 40.0, 30.0]),
                ("i1", [80.0, 100.0, 50.0, 30.0]),
                ("i2", [140.0, 130.0, 30.0, 10.0]),
            ],
        ),
    ]);
}
