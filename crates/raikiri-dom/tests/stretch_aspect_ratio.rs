//! A box with an `aspect-ratio` and a `stretch` size: the stretch size is
//! definite when the room it fills is, so the other size follows from it
//! through the ratio (CSS Sizing 4 §5.1), and no bound that the ratio carries
//! across from the other axis overrides it.
//!
//! * A block, float, flow-root or inline-block with `height: stretch` takes its
//!   `auto` width from the height; a `min-height` or `max-height` (a length or
//!   `stretch`) carries across to the width, as `min-width` and `max-width`
//!   carry to the height, but not into a `stretch` size.
//! * A flex item with a `stretch` cross size has the flex base size and the
//!   automatic minimum size that the cross size gives through the ratio.
//! * The `fit-content` width limits of a replaced box follow a stretched
//!   height.
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
    a_stretch_height_gives_the_width_through_the_ratio_with_block_content:
        r#"<div style="height:100px"><div id=a style="aspect-ratio:2/1;height:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_height_gives_the_width_through_the_ratio_without_content:
        r#"<div style="height:100px"><div id=a style="aspect-ratio:2/1;height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_height_may_make_the_box_wider_than_its_parent_with_block_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_height_may_make_the_box_wider_than_its_parent_without_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_height_may_make_the_box_wider_than_a_narrow_parent_with_block_content:
        r#"<div style="height:100px;width:60px"><div id=a style="aspect-ratio:2/1;height:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_height_may_make_the_box_wider_than_a_narrow_parent_without_content:
        r#"<div style="height:100px;width:60px"><div id=a style="aspect-ratio:2/1;height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_tall_ratio_gives_a_narrow_box_with_block_content:
        r#"<div style="height:100px"><div id=a style="aspect-ratio:1/2;height:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 100.0))];
    a_tall_ratio_gives_a_narrow_box_without_content:
        r#"<div style="height:100px"><div id=a style="aspect-ratio:1/2;height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 100.0))];
    a_stretch_height_with_border_box_edges_with_block_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;border:5px solid;padding:3px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_height_with_border_box_edges_without_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;border:5px solid;padding:3px"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_height_with_content_box_edges_with_block_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;box-sizing:content-box;border:5px solid;padding:3px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 184.0, 100.0))];
    a_stretch_height_with_content_box_edges_without_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;box-sizing:content-box;border:5px solid;padding:3px"></div></div>"# =>
        [("a", (8.0, 8.0, 184.0, 100.0))];
    a_stretch_height_leaves_out_margins_the_parent_holds_in_with_block_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;margin:7px 11px 3px 2px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (10.0, 8.0, 200.0, 100.0))];
    a_stretch_height_leaves_out_margins_the_parent_holds_in_without_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;margin:7px 11px 3px 2px"></div></div>"# =>
        [("a", (10.0, 8.0, 200.0, 100.0))];
    a_max_width_does_not_shrink_a_stretch_height_with_block_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;max-width:120px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 120.0, 100.0))];
    a_max_width_does_not_shrink_a_stretch_height_without_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;max-width:120px"></div></div>"# =>
        [("a", (8.0, 8.0, 120.0, 100.0))];
    a_min_width_does_not_grow_a_stretch_height_with_block_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;min-width:300px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 100.0))];
    a_min_width_does_not_grow_a_stretch_height_without_content:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;min-width:300px"></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 100.0))];
    a_stretch_height_without_a_definite_parent_height_is_auto_with_block_content:
        r#"<div style="width:100px"><div id=a style="aspect-ratio:2/1;height:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_stretch_height_without_a_definite_parent_height_is_auto_without_content:
        r#"<div style="width:100px"><div id=a style="aspect-ratio:2/1;height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_floated_stretch_height_with_block_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;float:left"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_floated_stretch_height_without_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;float:left"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_flow_root_stretch_height_with_block_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;display:flow-root"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_flow_root_stretch_height_without_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;display:flow-root"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_clipping_stretch_height_with_block_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;overflow:hidden"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_clipping_stretch_height_without_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;overflow:hidden"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_height_with_a_zero_min_width_with_block_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;min-width:0"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_height_with_a_zero_min_width_without_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;min-width:0"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_min_height_carries_to_the_width_with_block_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;min-height:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_min_height_carries_to_the_width_without_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;min-height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_max_height_carries_to_the_width_with_block_content:
        r#"<div style="height:100px"><div id=a style="aspect-ratio:2/1;max-height:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_max_height_carries_to_the_width_without_content:
        r#"<div style="height:100px"><div id=a style="aspect-ratio:2/1;max-height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_stretch_max_height_below_the_width_carries_to_it_with_block_content:
        r#"<div style="height:40px;width:100px"><div id=a style="aspect-ratio:2/1;max-height:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 80.0, 40.0))];
    a_stretch_max_height_below_the_width_carries_to_it_without_content:
        r#"<div style="height:40px;width:100px"><div id=a style="aspect-ratio:2/1;max-height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 80.0, 40.0))];
    a_min_height_length_carries_to_the_width_with_block_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;min-height:100px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_min_height_length_carries_to_the_width_without_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;min-height:100px"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    a_max_height_length_carries_to_the_width_with_block_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;max-height:40px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 80.0, 40.0))];
    a_max_height_length_carries_to_the_width_without_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;max-height:40px"></div></div>"# =>
        [("a", (8.0, 8.0, 80.0, 40.0))];
    a_max_height_does_not_shrink_a_stretch_width:
        r#"<div style="width:100px"><div id=a style="aspect-ratio:2/1;width:stretch;max-height:20px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 20.0))];
    a_min_height_does_not_grow_a_stretch_width:
        r#"<div style="width:100px"><div id=a style="aspect-ratio:2/1;width:stretch;min-height:80px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 80.0))];
    a_max_height_carries_to_an_auto_width:
        r#"<div style="width:100px"><div id=a style="aspect-ratio:2/1;max-height:20px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0))];
    a_stretch_height_with_max_width_below_the_ratio_width:
        r#"<div style="height:100px;width:150px"><div id=a style="aspect-ratio:2/1;height:stretch;max-width:100px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_stretch_height_raised_by_min_height:
        r#"<div style="height:100px"><div id=a style="aspect-ratio:2/1;height:stretch;min-height:150px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 150.0))];
    a_stretch_height_capped_by_max_height:
        r#"<div style="height:100px"><div id=a style="aspect-ratio:2/1;height:stretch;max-height:50px"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_stretch_width_in_a_definite_width_takes_its_height_from_the_ratio:
        r#"<div style="width:100px"><div id=a style="aspect-ratio:2/1;width:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_stretch_min_height_without_room_is_ignored:
        r#"<div style="width:100px"><div id=a style="aspect-ratio:2/1;min-height:stretch"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_stretch_height_grows_the_box_to_its_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch"><div style="width:300px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 100.0))];
    an_inline_block_with_a_stretch_height_and_block_content:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;display:inline-block"><div style="width:30px;height:40px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    an_empty_inline_block_with_a_stretch_height:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:stretch;display:inline-block"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    an_empty_inline_block_after_text:
        r#"<div style="height:100px;width:300px"><span>x</span><div id=a style="aspect-ratio:2/1;height:stretch;display:inline-block"></div></div>"# =>
        [("a", (18.0, 8.0, 200.0, 100.0))];
    an_empty_inline_block_with_limits:
        r#"<div style="height:100px;width:300px;font:20px/1 Ahem"><div id=a style="aspect-ratio:2/1;height:stretch;display:inline-block;vertical-align:top;min-width:50px;max-width:150px"></div></div>"# =>
        [("a", (8.0, 8.0, 150.0, 100.0))];
    an_empty_inline_block_with_edges:
        r#"<div style="height:100px;width:300px;font:20px/1 Ahem"><div id=a style="aspect-ratio:2/1;height:stretch;display:inline-block;vertical-align:top;border:5px solid;padding:3px"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    an_inline_block_with_a_length_height:
        r#"<div style="height:100px;width:100px"><div id=a style="aspect-ratio:2/1;height:100px;display:inline-block"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    column_replaced_item_with_a_stretch_width_has_a_flex_base_size_from_the_ratio:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;"><canvas id=a width=100 height=100 style="width:stretch;align-self:start;min-height:0"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    row_replaced_item_with_a_stretch_height_has_a_flex_base_size_from_the_ratio:
        r#"<div style="display:flex;flex-direction:row;height:200px;width:200px;"><canvas id=a width=100 height=100 style="height:stretch;align-self:start;min-width:0"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    column_replaced_item_with_a_stretch_width_and_a_zero_flex_basis_has_an_automatic_minimum_size:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;"><canvas id=a width=100 height=100 style="width:stretch;align-self:start;flex-basis:0"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    row_replaced_item_with_a_stretch_height_and_a_zero_flex_basis_has_an_automatic_minimum_size:
        r#"<div style="display:flex;flex-direction:row;height:200px;width:200px;"><canvas id=a width=100 height=100 style="height:stretch;align-self:start;flex-basis:0"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    column_block_item_with_a_stretch_width:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;"><div id=a style="aspect-ratio:1/1;width:stretch;align-self:start;min-height:0"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    row_block_item_with_a_stretch_height:
        r#"<div style="display:flex;flex-direction:row;height:200px;width:200px;"><div id=a style="aspect-ratio:1/1;height:stretch;align-self:start;min-width:0"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    column_block_item_with_a_stretch_width_and_a_zero_flex_basis:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;"><div id=a style="aspect-ratio:1/1;width:stretch;align-self:start;flex-basis:0"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    row_block_item_with_a_stretch_height_and_a_zero_flex_basis:
        r#"<div style="display:flex;flex-direction:row;height:200px;width:200px;"><div id=a style="aspect-ratio:1/1;height:stretch;align-self:start;flex-basis:0"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    column_replaced_item_stretched_by_align_self:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;"><canvas id=a width=100 height=100 style="width:stretch;min-height:0"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    column_replaced_item_with_margins:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;"><canvas id=a width=100 height=100 style="width:stretch;align-self:start;margin:0 10px 0 20px;min-height:0"></canvas></div>"# =>
        [("a", (28.0, 8.0, 170.0, 170.0))];
    column_replaced_item_in_a_padded_container:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;padding:7px"><canvas id=a width=100 height=100 style="width:stretch;align-self:start;min-height:0"></canvas></div>"# =>
        [("a", (15.0, 15.0, 186.0, 186.0))];
    column_replaced_item_with_an_automatic_minimum_size:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;"><canvas id=a width=100 height=100 style="width:stretch;align-self:start"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    column_replaced_item_in_a_wrapping_container:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;flex-wrap:wrap"><canvas id=a width=100 height=100 style="width:stretch;align-self:start;min-height:0"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    column_replaced_item_with_content_box_edges:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;"><canvas id=a width=100 height=100 style="width:stretch;align-self:start;min-height:0;box-sizing:content-box;border:5px solid;padding:3px"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    column_replaced_item_with_a_max_width:
        r#"<div style="display:flex;flex-direction:column;height:200px;width:200px;"><canvas id=a width=100 height=100 style="width:stretch;align-self:start;min-height:0;max-width:150px"></canvas></div>"# =>
        [("a", (8.0, 8.0, 150.0, 150.0))];
    column_replaced_item_shrunk_by_a_short_container:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:200px"><canvas id=a width=100 height=100 style="width:stretch;align-self:start;min-height:0"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    column_replaced_item_kept_whole_by_its_automatic_minimum_in_a_short_container:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:200px"><canvas id=a width=100 height=100 style="width:stretch;align-self:start"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    column_replaced_item_in_a_container_with_an_auto_height:
        r#"<div style="display:flex;flex-direction:column;width:200px"><canvas id=a width=100 height=100 style="width:stretch;align-self:start"></canvas></div>"# =>
        [("a", (8.0, 8.0, 200.0, 200.0))];
    a_replaced_box_with_a_stretch_height_takes_its_width_from_the_ratio:
        r#"<div style="width:20px;height:20px;margin:5px;border:1px solid black;box-sizing:content-box"><canvas id=a width=30 height=60 style="height:stretch;"></canvas></div>"# =>
        [("a", (14.0, 9.0, 10.0, 20.0))];
    a_replaced_min_width_fit_content_follows_a_stretch_height:
        r#"<div style="width:20px;height:20px;margin:5px;border:1px solid black;box-sizing:content-box"><canvas id=a width=30 height=60 style="height:stretch;width:1px;min-width:fit-content"></canvas></div>"# =>
        [("a", (14.0, 9.0, 10.0, 20.0))];
    a_replaced_max_width_fit_content_follows_a_stretch_height:
        r#"<div style="width:20px;height:20px;margin:5px;border:1px solid black;box-sizing:content-box"><canvas id=a width=30 height=60 style="height:stretch;width:100px;max-width:fit-content"></canvas></div>"# =>
        [("a", (14.0, 9.0, 10.0, 20.0))];
    a_replaced_fit_content_with_border:
        r#"<div style="width:20px;height:20px;margin:5px;border:1px solid black;box-sizing:content-box"><canvas id=a width=30 height=60 style="height:stretch;width:1px;min-width:fit-content;border-width:2px 3px 4px 5px;border-style:solid"></canvas></div>"# =>
        [("a", (14.0, 9.0, 15.0, 20.0))];
    a_replaced_fit_content_with_padding:
        r#"<div style="width:20px;height:20px;margin:5px;border:1px solid black;box-sizing:content-box"><canvas id=a width=30 height=60 style="height:stretch;width:100px;max-width:fit-content;padding:2px 3px 4px 5px"></canvas></div>"# =>
        [("a", (14.0, 9.0, 15.0, 20.0))];
    a_replaced_fit_content_with_margins:
        r#"<div style="width:20px;height:20px;margin:5px;border:1px solid black;box-sizing:content-box"><canvas id=a width=30 height=60 style="height:stretch;width:1px;min-width:fit-content;margin:1px 2px 3px 4px"></canvas></div>"# =>
        [("a", (18.0, 10.0, 8.0, 16.0))];
    a_replaced_fit_content_with_percentage_margins:
        r#"<div style="width:20px;height:20px;margin:5px;border:1px solid black;box-sizing:content-box"><canvas id=a width=30 height=60 style="height:stretch;width:100px;max-width:fit-content;margin:5% 10% 15% 20%"></canvas></div>"# =>
        [("a", (18.0, 10.0, 8.0, 16.0))];
    a_replaced_fit_content_without_a_definite_height_keeps_the_natural_size:
        r#"<div style="width:20px;margin:5px;border:1px solid black;box-sizing:content-box"><canvas id=a width=30 height=60 style="height:stretch;width:1px;min-width:fit-content"></canvas></div>"# =>
        [("a", (14.0, 9.0, 30.0, 60.0))];
}
