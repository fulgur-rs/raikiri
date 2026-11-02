//! A flex item with an `aspect-ratio` that its container stretches across the
//! main axis (an `auto` size across with `align-self: stretch`, or a `stretch`
//! one) has that size when the flex base size and the automatic minimum size
//! are resolved: the flex container stretches it before the lines form, and the
//! size is definite (CSS Flexbox 1 §9.4, CSS Sizing 4 §5.1).
//!
//! * The automatic minimum size along the main axis of a row item is the
//!   min-content width of its content, which the `max-height` that cuts the
//!   stretched height does not shrink through the ratio, and along the main
//!   axis of a column item the content height cut by what a `max-width` carries
//!   across.
//! * An item that grows is not held to the width the ratio gives, whatever
//!   `min-*` or `max-*` it states.
//! * A container that wraps, has no definite size across, or aligns the item
//!   otherwise, leaves the item at the size the ratio gives.
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

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

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

fn check(body: &str, expected: &[(&str, Rect)]) {
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

macro_rules! cases {
    ($($name:ident: $body:expr => [$(($id:expr, $rect:expr)),+ $(,)?];)+) => {
        $(
            #[test]
            fn $name() {
                check($body, &[$(($id, $rect)),+]);
            }
        )+
    };
}

cases! {
    a_stretched_row_item_cut_by_a_max_height_keeps_its_content_width:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_stretched_row_item_without_a_max_height_takes_its_width_from_the_ratio:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 200.0))];
    a_wide_ratio_row_item_cut_by_a_max_height_keeps_its_content_width:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:50px;aspect-ratio:2/1"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_min_height_above_the_container_height_sets_the_stretched_height:
        r#"<div style="display:flex;width:0;height:20px"><div id=a style="min-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    min_and_max_height_bound_the_stretched_height_of_a_row_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="min-height:30px;max-height:60px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 60.0))];
    a_container_shorter_than_the_max_height_stretches_the_item_to_it:
        r#"<div style="display:flex;width:0;height:30px"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 30.0))];
    a_row_container_with_an_auto_height_leaves_the_item_at_its_ratio_width:
        r#"<div style="display:flex;width:0"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 25.0, 50.0))];
    a_centered_row_item_is_not_stretched:
        r#"<div style="display:flex;width:0;height:200px;align-items:center"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 83.0, 25.0, 50.0))];
    align_self_stretch_stretches_a_row_item_of_a_centered_container:
        r#"<div style="display:flex;width:0;height:200px;align-items:center"><div id=a style="align-self:stretch;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    the_cross_margins_come_off_the_stretched_height_of_a_row_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="margin:5px 0;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 13.0, 100.0, 50.0))];
    an_auto_cross_margin_keeps_a_row_item_from_stretching:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="margin-top:auto;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 158.0, 25.0, 50.0))];
    a_zero_min_width_lets_the_stretched_row_item_shrink_to_nothing:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="min-width:0;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 0.0, 50.0))];
    a_scroll_container_row_item_has_no_content_minimum:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="overflow:hidden;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 0.0, 50.0))];
    a_percentage_max_height_cuts_the_stretched_height_of_a_row_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:25%;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    padding_adds_to_the_content_width_of_a_stretched_row_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="padding:3px;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 106.0, 50.0))];
    a_content_box_row_item_adds_padding_to_both_sizes:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="box-sizing:content-box;padding:3px;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 106.0, 56.0))];
    a_growing_stretched_row_item_is_not_held_to_the_ratio_width:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_wrapping_row_container_leaves_the_item_at_its_ratio_width:
        r#"<div style="display:flex;width:0;height:200px;flex-wrap:wrap"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 25.0, 50.0))];
    a_stretched_column_item_cut_by_a_max_width_is_held_to_the_ratio_height:
        r#"<div style="display:flex;flex-direction:column;height:0;width:200px"><div id=a style="max-width:50px;aspect-ratio:2/1"><div style="width:10px;height:100px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 25.0))];
    a_stretched_column_item_without_a_max_width_takes_its_height_from_the_ratio:
        r#"<div style="display:flex;flex-direction:column;height:0;width:200px"><div id=a style="aspect-ratio:2/1"><div style="width:10px;height:100px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_column_container_with_an_auto_width_stretches_nothing_across:
        r#"<div style="display:flex;flex-direction:column;height:0"><div id=a style="max-width:50px;aspect-ratio:2/1"><div style="width:10px;height:100px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 25.0))];
    a_growing_stretched_column_item_is_not_held_to_the_ratio_height:
        r#"<div style="display:flex;flex-direction:column;height:300px;width:200px"><div id=a style="flex-grow:1;max-width:50px;aspect-ratio:2/1"><div style="width:10px;height:100px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 300.0))];
    an_inline_flex_container_stretches_its_row_item_the_same_way:
        r#"<div style="display:flex;display:inline-flex;width:0;height:200px"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_stated_width_below_the_content_width_caps_the_minimum:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="width:20px;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 20.0, 50.0))];
    a_stated_width_above_the_content_width_shrinks_to_the_content:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="width:200px;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_max_width_above_the_ratio_width_grows_the_item_to_it:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-width:60px;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 50.0))];
    a_max_width_below_the_ratio_width_caps_the_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-width:10px;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 10.0, 50.0))];
    a_stretch_keyword_cross_size_gives_the_same_minimum:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="height:stretch;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    flex_grow_takes_the_item_past_a_max_height_cut_ratio_width:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex:1;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_fixed_flex_item_keeps_the_content_width_as_its_minimum:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex:0 0 auto;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    an_item_that_does_not_shrink_keeps_the_content_width:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="flex-shrink:0;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    content_narrower_than_the_ratio_width_leaves_the_ratio_width:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:10px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 25.0, 50.0))];
    align_items_normal_stretches_a_row_item:
        r#"<div style="display:flex;width:0;height:200px;align-items:normal"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    align_items_stretch_stretches_a_row_item:
        r#"<div style="display:flex;width:0;height:200px;align-items:stretch"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    flex_start_alignment_does_not_stretch_a_row_item:
        r#"<div style="display:flex;width:0;height:200px;align-items:flex-start"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 25.0, 50.0))];
    align_self_flex_start_does_not_stretch_a_row_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="align-self:flex-start;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 25.0, 50.0))];
    a_reversed_row_container_stretches_the_item_the_same_way:
        r#"<div style="display:flex;width:0;height:200px;flex-direction:row-reverse"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (-92.0, 8.0, 100.0, 50.0))];
    a_container_with_padding_stretches_to_its_content_height:
        r#"<div style="display:flex;width:0;height:210px;padding:5px"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (13.0, 13.0, 100.0, 50.0))];
    a_content_box_container_with_a_border_stretches_to_its_content_height:
        r#"<div style="display:flex;box-sizing:content-box;width:0;height:200px;border:5px solid"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (13.0, 13.0, 100.0, 50.0))];
    percentage_margins_come_off_the_stretched_height_of_a_row_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="margin:10% 0;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    margins_along_the_main_axis_do_not_change_the_stretched_height:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="margin:0 7px;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (15.0, 8.0, 100.0, 50.0))];
    a_three_to_one_ratio_gives_a_wider_stretched_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:50px;aspect-ratio:3/1"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 150.0, 50.0))];
    an_auto_ratio_keeps_the_content_width_as_the_minimum:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:50px;aspect-ratio:auto 1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_zero_min_height_does_not_change_the_stretched_row_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:50px;min-height:0;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_min_height_above_the_max_height_wins:
        r#"<div style="display:flex;width:0;height:20px"><div id=a style="min-height:60px;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 60.0))];
    a_flex_container_inside_the_stretched_item_gives_the_same_minimum:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:50px;aspect-ratio:1/2"><div style="display:flex"><div style="width:100px;height:10px"></div></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    text_in_the_stretched_item_gives_the_same_minimum:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:50px;aspect-ratio:1/2"><span>abcdefghij</span></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_vertical_item_is_left_to_its_own_axes:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="max-height:50px;aspect-ratio:1/2;writing-mode:vertical-rl"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 25.0, 50.0))];
    a_centered_column_item_is_stretched_to_its_max_width:
        r#"<div style="display:flex;flex-direction:column;height:0;width:200px;align-items:center"><div id=a style="max-width:50px;aspect-ratio:2/1"><div style="width:10px;height:100px"></div></div></div>"# =>
        [("a", (83.0, 8.0, 50.0, 25.0))];
    a_column_container_with_padding_stretches_to_its_content_width:
        r#"<div style="display:flex;flex-direction:column;height:0;width:210px;padding:5px"><div id=a style="max-width:50px;aspect-ratio:2/1"><div style="width:10px;height:100px"></div></div></div>"# =>
        [("a", (13.0, 13.0, 50.0, 25.0))];
    a_min_width_above_the_max_width_wins_in_a_column:
        r#"<div style="display:flex;flex-direction:column;height:0;width:200px"><div id=a style="min-width:80px;max-width:50px;aspect-ratio:2/1"><div style="width:10px;height:100px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 80.0, 40.0))];
    a_column_item_with_a_stated_height_is_still_held_to_the_ratio_height:
        r#"<div style="display:flex;flex-direction:column;height:0;width:200px"><div id=a style="height:200px;max-width:50px;aspect-ratio:2/1"><div style="width:10px;height:100px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 25.0))];
    a_growing_item_with_a_stated_height_grows_past_the_ratio_width:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_growing_item_with_a_zero_min_width_and_a_max_height_grows:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;max-height:50px;aspect-ratio:1/2;min-width:0"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_growing_item_with_zero_min_sizes_and_a_max_height_grows:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;max-height:50px;aspect-ratio:1/2;min-width:0;min-height:0"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_growing_item_with_a_zero_flex_basis_and_a_max_height_grows:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;max-height:50px;aspect-ratio:1/2;min-width:0;flex-basis:0"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_growing_item_with_a_max_width_grows:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;max-height:50px;aspect-ratio:1/2;min-width:20px;max-width:1000px"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_growing_item_with_an_auto_height_and_a_min_width_grows:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;max-height:50px;aspect-ratio:1/2;min-width:20px;min-height:0;height:auto"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_growing_item_with_equal_height_and_max_height_grows:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;height:50px;max-height:50px;aspect-ratio:1/2;min-width:20px"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_growing_item_with_a_height_below_its_max_height_grows:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;height:50px;max-height:60px;aspect-ratio:1/2;min-width:20px"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];
    a_growing_start_aligned_item_with_a_max_height_grows:
        r#"<div style="display:flex;width:300px;height:200px"><div id=a style="flex-grow:1;max-height:50px;aspect-ratio:1/2;min-width:20px;align-self:flex-start"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 50.0))];

    a_min_content_height_row_item_stays_at_its_ratio_width:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="height:min-content;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 25.0, 50.0))];
    a_fit_content_height_row_item_stays_at_its_ratio_width:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="height:fit-content;max-height:50px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 25.0, 50.0))];
    a_percentage_height_is_the_stretched_height_of_a_row_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="height:50%;max-height:80px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 80.0))];
    a_max_content_height_row_item_takes_its_width_from_the_ratio:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="height:max-content;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 200.0))];
    a_calc_height_is_the_stretched_height_of_a_row_item:
        r#"<div style="display:flex;width:0;height:200px"><div id=a style="height:calc(50% + 10px);max-height:80px;aspect-ratio:1/2"><div style="width:100px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 80.0))];
    a_min_content_width_column_item_stays_at_its_ratio_height:
        r#"<div style="display:flex;flex-direction:column;height:0;width:200px"><div id=a style="width:min-content;max-width:50px;aspect-ratio:2/1"><div style="width:10px;height:100px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 25.0))];
}

#[test]
fn a_percentage_main_size_survives_a_stretched_ratio_items_cross_maximum() {
    check(
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px"><div style="height:30px"></div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 100.0, 50.0)),
            ("d", (8.0, 8.0, 100.0, 20.0)),
        ],
    );
}

#[test]
fn a_percentage_main_size_with_a_cross_maximum_needs_no_ratio() {
    check(
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;max-height:20px"><div style="height:30px"></div></div></div>"#,
        &[
            ("c", (8.0, 8.0, 100.0, 50.0)),
            ("d", (8.0, 8.0, 100.0, 20.0)),
        ],
    );
}

cases! {
    main_cross_original:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_fixed_height:
        r#"<div id=c style="display:flex;width:100px;height:50px"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_wrap:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_center:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1;align-items:center"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 23.0, 100.0, 20.0))];
    main_cross_start:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1;align-items:start"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_row_reverse:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1;flex-direction:row-reverse"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_fixed_width:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100px;aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_auto_width:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_calc_width:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:calc(100% - 10px);aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 90.0, 20.0))];
    main_cross_no_ratio:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;max-height:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_basis_content:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;flex-basis:content"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_basis_zero:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;flex-basis:0px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_basis_fixed:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;flex-basis:40px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 40.0, 20.0))];
    main_cross_basis_max_content:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;flex-basis:max-content"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_min_height:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;min-height:30px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 30.0))];
    main_cross_percent_max:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;max-height:50%"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 25.0))];
    main_cross_padding:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;padding:3px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_border:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;border:2px solid"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_content_box:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;box-sizing:content-box;padding:3px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 26.0))];
    main_cross_auto_cross_margin:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;margin-top:auto"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 38.0, 100.0, 20.0))];
    main_cross_cross_margins:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;margin:5px 0"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 13.0, 100.0, 20.0))];
    main_cross_zero_min:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;min-width:0px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_overflow:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;overflow:hidden"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_self_stretch:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;align-self:stretch"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_grow:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;flex-grow:1"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 100.0, 20.0))];
    main_cross_column:
        r#"<div id=c style="display:flex;flex-direction:column;height:100px;aspect-ratio:1/2"><div id=d style="height:100%;aspect-ratio:1/1;max-width:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 50.0, 100.0)), ("d", (8.0, 8.0, 20.0, 100.0))];
    main_cross_column_content:
        r#"<div id=c style="display:flex;flex-direction:column;height:100px;aspect-ratio:1/2"><div id=d style="height:100%;aspect-ratio:1/1;max-width:20px;flex-basis:content"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 50.0, 100.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
}

cases! {
    main_cross_auto_main_basis_0:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="aspect-ratio:1/1;max-height:20px;flex-basis:0px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_column_auto_main_basis_0:
        r#"<div id=c style="display:flex;flex-direction:column;height:100px;aspect-ratio:1/2"><div id=d style="aspect-ratio:1/1;max-width:20px;flex-basis:0px"><div style="height:10px;width:30px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 50.0, 100.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_auto_main_basis_10:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="aspect-ratio:1/1;max-height:20px;flex-basis:10px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_column_auto_main_basis_10:
        r#"<div id=c style="display:flex;flex-direction:column;height:100px;aspect-ratio:1/2"><div id=d style="aspect-ratio:1/1;max-width:20px;flex-basis:10px"><div style="height:10px;width:30px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 50.0, 100.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_auto_main_basis_40:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="aspect-ratio:1/1;max-height:20px;flex-basis:40px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 40.0, 20.0))];
    main_cross_column_auto_main_basis_40:
        r#"<div id=c style="display:flex;flex-direction:column;height:100px;aspect-ratio:1/2"><div id=d style="aspect-ratio:1/1;max-width:20px;flex-basis:40px"><div style="height:10px;width:30px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 50.0, 100.0)), ("d", (8.0, 8.0, 20.0, 40.0))];
}

cases! {
    main_cross_resolved_percent_basis:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;flex-basis:40%"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 40.0, 20.0))];
    main_cross_resolved_calc_basis:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;flex-basis:calc(50% - 10px)"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 40.0, 20.0))];
    main_cross_cyclic_percent_basis:
        r#"<div id=c style="display:inline-flex;height:50px"><div id=d style="aspect-ratio:1/1;max-height:20px;flex-basis:40%"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 20.0, 50.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_cyclic_column_percent_basis:
        r#"<div id=c style="display:inline-flex;flex-direction:column;width:50px"><div id=d style="aspect-ratio:1/1;max-width:20px;flex-basis:40%"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 50.0, 20.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_column_reverse:
        r#"<div id=c style="display:flex;flex-direction:column-reverse;height:100px;aspect-ratio:1/2"><div id=d style="height:100%;aspect-ratio:1/1;max-width:20px"><div style="height:30px;width:10px"></div></div></div>"# =>
        [("c", (8.0, 8.0, 50.0, 100.0)), ("d", (8.0, 8.0, 20.0, 100.0))];
    main_cross_shrink_fixed_basis:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px;flex-basis:80px"><div style="height:30px;width:10px"></div></div><div style="width:80px;flex:none"></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
    main_cross_shrink_auto_basis:
        r#"<div id=c style="display:flex;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;max-height:20px"><div style="height:30px;width:10px"></div></div><div style="width:80px;flex:none"></div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 50.0)), ("d", (8.0, 8.0, 20.0, 20.0))];
}
