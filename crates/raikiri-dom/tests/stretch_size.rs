//! `stretch` (and its alias `-webkit-fill-available`) as the size of a box
//! (CSS Sizing 4 §5.1): the margin box fills the containing block. The inline
//! axis subtracts the margins; the block axis needs a containing block whose
//! height is definite, takes a margin off only where the parent holds it in
//! (a box that roots its own formatting context, or one with a border or
//! padding on that side), and otherwise behaves as `auto`.
//!
//! An auto margin counts as zero when `stretch` sizes the box, so the box
//! fills the room even beside `margin: 0 auto`; what a `max-width` then
//! leaves over is shared by the auto margins. The same holds for an
//! absolutely positioned box between its insets.
//!
//! An absolutely positioned box with `auto` insets in an axis sits at its
//! static position, and `stretch` there fills the room from that position to
//! the far edge of the containing block, not the whole block (which the box
//! would overflow by the offset of its static position). Where the static
//! position is centered in the containing block, the box fills it.
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
    inline_axis_block_fills_room_less_margins:
        r#"<div style="width:200px"><div id=a style="width:stretch;margin:0 10px 0 20px"></div></div>"# =>
        [("a", (28.0, 8.0, 170.0, 0.0))];
    inline_axis_webkit_alias:
        r#"<div style="width:200px"><div id=a style="width:-webkit-fill-available;margin:0 10px 0 20px"></div></div>"# =>
        [("a", (28.0, 8.0, 170.0, 0.0))];
    inline_axis_logical_inline_size:
        r#"<div style="width:200px"><div id=a style="inline-size:stretch;margin:0 10px"></div></div>"# =>
        [("a", (18.0, 8.0, 180.0, 0.0))];
    inline_axis_padded_block:
        r#"<div style="width:200px"><div id=a style="width:stretch;margin:0 10px;padding:0 5px;border:2px solid"></div></div>"# =>
        [("a", (18.0, 8.0, 180.0, 4.0))];
    inline_axis_content_box_block:
        r#"<div style="width:200px"><div id=a style="box-sizing:content-box;width:stretch;margin:0 10px;padding:0 5px"></div></div>"# =>
        [("a", (18.0, 8.0, 180.0, 0.0))];
    inline_axis_nested_blocks:
        r#"<div style="width:200px"><div id=a style="width:stretch;padding:0 10px"><div id=b style="width:stretch"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 0.0)), ("b", (18.0, 8.0, 180.0, 0.0))];
    inline_axis_float_fills_room:
        r#"<div style="width:200px"><div id=a style="float:left;width:stretch;height:10px;margin:0 5px"></div></div>"# =>
        [("a", (13.0, 8.0, 190.0, 10.0))];
    inline_axis_empty_float_fills_room:
        r#"<div style="width:200px"><div id=a style="float:left;width:stretch;margin:0 5px"></div></div>"# =>
        [("a", (13.0, 8.0, 190.0, 0.0))];
    inline_axis_inline_block_fills_line:
        r#"<div style="width:200px"><span id=a style="display:inline-block;width:stretch;height:10px"></span></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    inline_axis_canvas_fills_room:
        r#"<div style="width:200px"><canvas id=a style="display:block;width:stretch;height:10px;margin:0 10px"></canvas></div>"# =>
        [("a", (18.0, 8.0, 180.0, 10.0))];
    inline_axis_fixed_table_fills_room:
        r#"<div style="width:200px"><div id=a style="display:table;table-layout:fixed;width:stretch;margin:0 10px"></div></div>"# =>
        [("a", (18.0, 8.0, 180.0, 0.0))];
    inline_axis_flex_item:
        r#"<div style="display:flex;width:200px"><div id=a style="width:stretch;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    inline_axis_grid_item:
        r#"<div style="display:grid;width:200px"><div id=a style="width:stretch;height:10px;margin:0 10px"></div></div>"# =>
        [("a", (18.0, 8.0, 180.0, 10.0))];
    inline_axis_shrink_wrapped_parent:
        r#"<div id=p style="float:left"><div id=a style="width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("p", (8.0, 8.0, 30.0, 10.0)), ("a", (8.0, 8.0, 30.0, 10.0))];
    inline_axis_min_content_container:
        r#"<div id=p style="float:left;width:min-content"><div id=a style="width:stretch">aa bbbb</div></div>"# =>
        [("p", (8.0, 8.0, 40.0, 20.0)), ("a", (8.0, 8.0, 40.0, 20.0))];
    inline_axis_max_content_container:
        r#"<div id=p style="float:left;width:max-content"><div id=a style="width:stretch">aa bbbb</div></div>"# =>
        [("p", (8.0, 8.0, 70.0, 10.0)), ("a", (8.0, 8.0, 70.0, 10.0))];
    inline_axis_table_in_shrink_wrapped_parent:
        r#"<div id=p style="float:left"><div id=a style="display:table;table-layout:fixed;width:stretch"><div style="display:table-cell;width:30px;height:10px"></div></div></div>"# =>
        [("p", (8.0, 8.0, 30.0, 10.0)), ("a", (8.0, 8.0, 30.0, 10.0))];
    inline_axis_min_content_block_children:
        r#"<div id=p style="float:left;width:min-content"><div id=a style="width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("p", (8.0, 8.0, 30.0, 10.0)), ("a", (8.0, 8.0, 30.0, 10.0))];
    inline_axis_min_content_nested_block_children:
        r#"<div id=p style="float:left;width:min-content"><div id=a style="width:stretch;padding:0 5px"><div id=b style="width:stretch"><div style="width:30px;height:10px"></div></div></div></div>"# =>
        [("p", (8.0, 8.0, 40.0, 10.0)), ("a", (8.0, 8.0, 40.0, 10.0)), ("b", (13.0, 8.0, 30.0, 10.0))];
    inline_axis_table_cell_min_content:
        r#"<div style="display:table"><div style="display:table-cell;width:0"><div id=a style="width:stretch"><div style="width:30px;height:10px"></div></div></div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 10.0))];
    inline_axis_inline_block_with_block_children_fills_line:
        r#"<div style="width:200px"><span id=a style="display:inline-block;width:stretch"><div style="width:30px;height:10px"></div></span></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    inline_axis_inline_block_with_block_children_less_margins:
        r#"<div style="width:200px">aa<span id=a style="display:inline-block;width:stretch;margin:0 10px"><div style="width:30px;height:10px"></div></span></div>"# =>
        [("a", (18.0, 18.0, 180.0, 10.0))];
    inline_axis_float_in_a_paragraph_fills_room:
        r#"<div style="width:200px"><div id=a style="float:left;width:stretch;margin:0 5px"><div style="width:30px;height:10px"></div></div>aa</div>"# =>
        [("a", (13.0, 8.0, 190.0, 10.0))];
    inline_axis_root_block:
        r#"<div id=a style="width:stretch;margin:0 10px"><div style="width:30px;height:10px"></div></div>"# =>
        [("a", (18.0, 8.0, 764.0, 10.0))];
    inline_axis_abspos_with_insets:
        r#"<div style="position:relative;width:200px;height:50px"><div id=a style="position:absolute;left:10px;right:20px;width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (18.0, 8.0, 170.0, 10.0))];
}

cases! {
    block_axis_fills_parent:
        r#"<div style="height:100px"><div id=a style="height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 100.0))];
    block_axis_logical_block_size:
        r#"<div style="height:100px"><div id=a style="block-size:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 100.0))];
    block_axis_webkit_alias:
        r#"<div style="height:100px"><div id=a style="height:-webkit-fill-available"></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 100.0))];
    block_axis_margins_that_escape_are_ignored:
        r#"<div id=w style="height:100px"><div id=a style="height:stretch;margin:5px 0 7px"></div></div>"# =>
        [("w", (8.0, 8.0, 784.0, 100.0)), ("a", (8.0, 8.0, 784.0, 100.0))];
    block_axis_border_top_holds_top_margin:
        r#"<div id=w style="height:100px;border-top:1px solid"><div id=a style="height:stretch;margin:5px 0 7px"></div></div>"# =>
        [("w", (8.0, 8.0, 784.0, 100.0)), ("a", (8.0, 14.0, 784.0, 94.0))];
    block_axis_border_bottom_holds_bottom_margin:
        r#"<div id=w style="height:100px;border-bottom:1px solid"><div id=a style="height:stretch;margin:5px 0 7px"></div></div>"# =>
        [("w", (8.0, 8.0, 784.0, 100.0)), ("a", (8.0, 8.0, 784.0, 92.0))];
    block_axis_padding_holds_both_margins:
        r#"<div id=w style="height:100px;padding:1px 0"><div id=a style="height:stretch;margin:5px 0 7px"></div></div>"# =>
        [("w", (8.0, 8.0, 784.0, 100.0)), ("a", (8.0, 14.0, 784.0, 86.0))];
    block_axis_formatting_context_holds_both_margins:
        r#"<div id=w style="height:100px;overflow:hidden"><div id=a style="height:stretch;margin:5px 0 7px"></div></div>"# =>
        [("w", (8.0, 8.0, 784.0, 100.0)), ("a", (8.0, 13.0, 784.0, 88.0))];
    block_axis_flow_root_holds_both_margins_of_a_middle_child:
        r#"<div style="height:100px;display:flow-root"><div style="height:10px"></div><div id=a style="height:stretch;margin:5px 0 7px"></div><div style="height:10px"></div></div>"# =>
        [("a", (8.0, 23.0, 784.0, 88.0))];
    block_axis_border_holds_both_margins_of_a_middle_child:
        r#"<div style="height:100px;border:1px solid"><div style="height:10px"></div><div id=a style="height:stretch;margin:5px 0 7px"></div><div style="height:10px"></div></div>"# =>
        [("a", (9.0, 24.0, 782.0, 86.0))];
    block_axis_middle_child_of_a_plain_parent:
        r#"<div style="height:100px"><div style="height:10px"></div><div id=a style="height:stretch;margin:5px 0 7px"></div><div style="height:10px"></div></div>"# =>
        [("a", (8.0, 23.0, 784.0, 100.0))];
    block_axis_padding_holds_the_margin_after_an_empty_sibling:
        r#"<div style="height:100px;padding-top:1px"><div style="margin:20px 0"></div><div id=a style="height:stretch;margin:5px 0 7px"></div></div>"# =>
        [("a", (8.0, 29.0, 784.0, 94.0))];
    block_axis_last_child_before_a_float:
        r#"<div style="height:100px"><div id=a style="display:flow-root;width:100px;height:stretch;margin-bottom:50px"></div><div style="float:left;width:50px;height:50px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    block_axis_float_takes_both_margins:
        r#"<div style="height:100px"><div id=a style="float:left;width:20px;height:stretch;margin:5px 0 7px"></div></div>"# =>
        [("a", (8.0, 13.0, 20.0, 88.0))];
    block_axis_inline_block_takes_both_margins:
        r#"<div style="height:100px"><span id=a style="display:inline-block;height:stretch;width:20px;margin:5px 0 7px"></span></div>"# =>
        [("a", (8.0, 13.0, 20.0, 88.0))];
    block_axis_content_box_sizing:
        r#"<div style="height:100px"><div id=a style="box-sizing:content-box;padding:10px;height:stretch"></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 100.0))];
    block_axis_max_height_limits:
        r#"<div style="height:100px"><div id=a style="height:stretch;max-height:50px"></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 50.0))];
    block_axis_min_height_raises:
        r#"<div style="height:100px"><div id=a style="height:stretch;min-height:150px"></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 150.0))];
    block_axis_nested_stretch_and_percent:
        r#"<div style="height:100px"><div id=a style="height:stretch"><div id=b style="height:stretch"></div><div id=c style="height:50%"></div></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 100.0)), ("b", (8.0, 8.0, 784.0, 100.0)), ("c", (8.0, 108.0, 784.0, 50.0))];
    block_axis_indefinite_parent_acts_as_auto:
        r#"<div id=w><div id=a style="height:stretch"><div style="height:20px"></div></div></div>"# =>
        [("w", (8.0, 8.0, 784.0, 20.0)), ("a", (8.0, 8.0, 784.0, 20.0))];
    block_axis_percent_parent_indefinite_acts_as_auto:
        r#"<div style="height:50%"><div id=a style="height:stretch"><div style="height:20px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 20.0))];
    block_axis_canvas:
        r#"<div style="height:100px"><canvas id=a style="display:block;height:stretch;width:30px;margin:5px 0 7px"></canvas></div>"# =>
        [("a", (8.0, 8.0, 30.0, 100.0))];
    block_axis_relative_offset:
        r#"<div style="height:100px"><div id=a style="position:relative;top:3px;height:stretch"></div></div>"# =>
        [("a", (8.0, 11.0, 784.0, 100.0))];
    block_axis_table:
        r#"<div style="height:100px"><div id=a style="display:table;height:stretch;margin:5px 0 7px"></div></div>"# =>
        [("a", (8.0, 8.0, 0.0, 100.0))];
    block_axis_flex_container:
        r#"<div style="height:100px"><div id=a style="display:flex;height:stretch"><div id=b style="width:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 100.0)), ("b", (8.0, 8.0, 10.0, 100.0))];
    block_axis_grid_container:
        r#"<div style="height:100px"><div id=a style="display:grid;height:stretch"><div id=b style="width:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 784.0, 100.0)), ("b", (8.0, 8.0, 10.0, 100.0))];
    block_axis_float_in_inline_root:
        r#"<div style="height:100px">aa<div id=a style="float:left;width:20px;height:stretch;margin:5px 0 7px"></div></div>"# =>
        [("a", (8.0, 13.0, 20.0, 88.0))];
    block_axis_unresolved_height_lets_the_bottom_margin_escape:
        r#"<div id=w><div id=a style="height:stretch;margin-bottom:10px"><div style="height:20px"></div></div></div><div id=b style="height:5px"></div>"# =>
        [("w", (8.0, 8.0, 784.0, 20.0)), ("a", (8.0, 8.0, 784.0, 20.0)), ("b", (8.0, 38.0, 784.0, 5.0))];
    block_axis_bottom_margin_held_by_a_definite_parent_with_a_next_sibling:
        r#"<div id=w style="height:100px"><div id=a style="height:stretch;margin-bottom:10px"></div></div><div id=b style="height:5px"></div>"# =>
        [("w", (8.0, 8.0, 784.0, 100.0)), ("a", (8.0, 8.0, 784.0, 100.0)), ("b", (8.0, 108.0, 784.0, 5.0))];
}

cases! {
    aspect_ratio_unresolved_height_follows_the_ratio:
        r#"<div style="width:200px"><div id=a style="width:30px;height:stretch;aspect-ratio:1"></div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 30.0))];
    aspect_ratio_unresolved_height_is_not_raised_by_content:
        r#"<div style="width:200px"><div id=a style="width:30px;height:stretch;aspect-ratio:1"><div style="height:50px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 30.0))];
    aspect_ratio_resolved_height_takes_the_room:
        r#"<div style="width:200px;height:100px"><div id=a style="width:30px;height:stretch;aspect-ratio:1"></div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 100.0))];
    aspect_ratio_resolved_height_ignores_the_content:
        r#"<div style="width:200px;height:100px"><div id=a style="width:30px;height:stretch;aspect-ratio:1"><div style="height:150px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 100.0))];
    aspect_ratio_resolved_height_less_held_margins:
        r#"<div style="width:200px;height:100px;overflow:hidden"><div id=a style="width:30px;height:stretch;margin:5px 0 15px;aspect-ratio:1"></div></div>"# =>
        [("a", (8.0, 13.0, 30.0, 80.0))];
    aspect_ratio_stretch_width_gives_the_unresolved_height:
        r#"<div style="width:200px"><div id=a style="width:stretch;height:stretch;aspect-ratio:2"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    aspect_ratio_stretch_both_resolved:
        r#"<div style="width:200px;height:100px"><div id=a style="width:stretch;height:stretch;aspect-ratio:2"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 100.0))];
    aspect_ratio_float_unresolved_height:
        r#"<div style="width:200px"><div id=a style="float:left;width:30px;height:stretch;aspect-ratio:1"></div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 30.0))];
    aspect_ratio_inline_block_unresolved_height:
        r#"<div style="width:200px"><span id=a style="display:inline-block;width:30px;height:stretch;aspect-ratio:1"></span></div>"# =>
        [("a", (8.0, 8.0, 30.0, 30.0))];
    aspect_ratio_flex_item_unresolved_height:
        r#"<div style="display:flex;width:200px"><div id=a style="width:30px;height:stretch;aspect-ratio:1"></div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 30.0))];
    aspect_ratio_flex_item_resolved_height:
        r#"<div style="display:flex;width:200px;height:100px"><div id=a style="width:30px;height:stretch;aspect-ratio:1"></div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 100.0))];
    aspect_ratio_flex_item_stretch_width:
        r#"<div style="display:flex;width:200px"><div id=a style="width:stretch;height:20px;aspect-ratio:1;flex:none"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 20.0))];
}

cases! {
    container_inline_flex_fills_room:
        r#"<div style="width:200px"><div id=a style="display:inline-flex;width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    container_inline_flex_less_margins:
        r#"<div style="width:200px"><div id=a style="display:inline-flex;width:stretch;margin:0 10px 0 20px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (28.0, 8.0, 170.0, 10.0))];
    container_inline_grid_fills_room:
        r#"<div style="width:200px"><div id=a style="display:inline-grid;width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    container_inline_grid_less_margins:
        r#"<div style="width:200px"><div id=a style="display:inline-grid;width:stretch;margin:0 10px 0 20px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (28.0, 8.0, 170.0, 10.0))];
    container_inline_flex_with_text:
        r#"<div style="width:200px"><div id=a style="display:inline-flex;width:stretch">aaa</div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    container_inline_flex_after_text:
        r#"<div style="width:200px">xx<div id=a style="display:inline-flex;width:stretch"><div style="width:30px;height:10px"></div></div>yy</div>"# =>
        [("a", (8.0, 18.0, 200.0, 10.0))];
    container_inline_flex_both_axes:
        r#"<div style="width:200px;height:100px"><div id=a style="display:inline-flex;width:stretch;height:stretch;margin:5px 0 15px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 13.0, 200.0, 80.0))];
    container_floated_flex_fills_room:
        r#"<div style="width:200px"><div id=a style="float:left;display:flex;width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    container_floated_grid_fills_room:
        r#"<div style="width:200px"><div id=a style="float:left;display:grid;width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    container_block_flex_less_margins:
        r#"<div style="width:200px"><div id=a style="display:flex;width:stretch;margin:0 10px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (18.0, 8.0, 180.0, 10.0))];
    container_block_grid_less_margins:
        r#"<div style="width:200px"><div id=a style="display:grid;width:stretch;margin:0 10px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (18.0, 8.0, 180.0, 10.0))];
    container_inline_flex_max_width_limits:
        r#"<div style="width:200px"><div id=a style="display:inline-flex;width:stretch;max-width:50px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 10.0))];
    container_inline_flex_min_width_raises:
        r#"<div style="width:200px"><div id=a style="display:inline-flex;width:stretch;min-width:250px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 250.0, 10.0))];
    container_floated_grid_max_width_limits:
        r#"<div style="width:200px"><div id=a style="float:left;display:grid;width:stretch;max-width:50px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 10.0))];
    container_floated_flex_min_width_raises:
        r#"<div style="width:200px"><div id=a style="float:left;display:flex;width:stretch;min-width:250px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 250.0, 10.0))];
    container_inline_flex_padding_border_and_margins:
        r#"<div style="width:200px"><div id=a style="display:inline-flex;width:stretch;padding:0 5px;border:2px solid;margin:0 3px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (11.0, 8.0, 194.0, 14.0))];
    container_inline_flex_content_box_sizing:
        r#"<div style="width:200px"><div id=a style="box-sizing:content-box;display:inline-flex;width:stretch;padding:0 5px;border:2px solid;margin:0 3px"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (11.0, 8.0, 194.0, 14.0))];
    container_flex_in_a_shrink_wrapped_parent:
        r#"<div id=p style="float:left"><div id=a style="display:flex;width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("p", (8.0, 8.0, 30.0, 10.0)), ("a", (8.0, 8.0, 30.0, 10.0))];
    container_flex_that_is_a_flex_item:
        r#"<div style="display:flex;width:200px"><div id=a style="display:flex;width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    container_grid_that_is_a_grid_item:
        r#"<div style="display:grid;width:200px"><div id=a style="display:grid;width:stretch"><div style="width:30px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
    container_inline_grid_with_text:
        r#"<div style="width:200px"><div id=a style="display:inline-grid;width:stretch">aaa</div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 10.0))];
}

cases! {
    limits_canvas_max_width:
        r#"<div style="width:200px"><canvas id=a style="display:block;width:stretch;max-width:50px;height:10px"></canvas></div>"# =>
        [("a", (8.0, 8.0, 50.0, 10.0))];
    limits_canvas_min_width:
        r#"<div style="width:200px"><canvas id=a style="display:block;width:stretch;min-width:250px;height:10px"></canvas></div>"# =>
        [("a", (8.0, 8.0, 250.0, 10.0))];
    limits_fixed_table_max_width:
        r#"<div style="width:200px"><div id=a style="display:table;table-layout:fixed;width:stretch;max-width:50px"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 0.0))];
    limits_fixed_table_min_width:
        r#"<div style="width:200px"><div id=a style="display:table;table-layout:fixed;width:stretch;min-width:250px"></div></div>"# =>
        [("a", (8.0, 8.0, 250.0, 0.0))];
    limits_block_max_width:
        r#"<div style="width:200px"><div id=a style="width:stretch;max-width:50px;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 10.0))];
    limits_block_min_width:
        r#"<div style="width:200px"><div id=a style="width:stretch;min-width:250px;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 250.0, 10.0))];
    limits_float_max_width:
        r#"<div style="width:200px"><div id=a style="float:left;width:stretch;max-width:50px;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 10.0))];
    limits_inline_block_max_width:
        r#"<div style="width:200px"><span id=a style="display:inline-block;width:stretch;max-width:50px;height:10px"></span></div>"# =>
        [("a", (8.0, 8.0, 50.0, 10.0))];
    limits_flex_item_max_width:
        r#"<div style="display:flex;width:200px"><div id=a style="width:stretch;max-width:50px;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 10.0))];
}

cases! {
    auto_margins_both:
        r#"<div style="width:100px"><div id=a style="margin:0 auto;width:stretch;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    auto_margins_left:
        r#"<div style="width:100px"><div id=a style="margin-left:auto;width:stretch;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    auto_margins_right:
        r#"<div style="width:100px"><div id=a style="margin-right:auto;width:stretch;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    auto_margin_beside_length_right:
        r#"<div style="width:100px"><div id=a style="margin:0 auto 0 10px;width:stretch;height:10px"></div></div>"# =>
        [("a", (18.0, 8.0, 90.0, 10.0))];
    auto_margin_beside_length_left:
        r#"<div style="width:100px"><div id=a style="margin:0 20px 0 auto;width:stretch;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 80.0, 10.0))];
    auto_margins_stretch_clamped_by_max_width:
        r#"<div style="width:100px"><div id=a style="margin:0 auto;width:stretch;max-width:50px;height:10px"></div></div>"# =>
        [("a", (33.0, 8.0, 50.0, 10.0))];
    auto_margins_max_width_stretch_clamps_width:
        r#"<div style="width:100px"><div id=a style="margin:0 auto;width:30px;max-width:stretch;height:10px"></div></div>"# =>
        [("a", (43.0, 8.0, 30.0, 10.0))];
    auto_margins_min_width_stretch_raises_width:
        r#"<div style="width:100px"><div id=a style="margin:0 auto;min-width:stretch;width:25px;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    auto_margins_min_width_stretch_beside_length:
        r#"<div style="width:100px"><div id=a style="margin:0 auto 0 10px;min-width:stretch;width:25px;height:10px"></div></div>"# =>
        [("a", (18.0, 8.0, 90.0, 10.0))];
    auto_margins_block_formatting_context:
        r#"<div style="width:100px"><div id=a style="margin:0 auto;width:stretch;height:10px;overflow:hidden"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    auto_margins_flex_item:
        r#"<div style="display:flex;width:100px"><div id=a style="margin:0 auto;width:stretch;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    auto_margins_grid_item:
        r#"<div style="display:grid;width:100px"><div id=a style="margin:0 auto;width:stretch;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    auto_margins_float:
        r#"<div style="width:100px"><div id=a style="float:left;margin:0 auto;width:stretch;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    auto_margins_inline_block:
        r#"<div style="width:100px"><div id=a style="display:inline-block;margin:0 auto;width:stretch;height:10px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    auto_margins_canvas:
        r#"<div style="width:100px"><canvas id=a style="display:block;margin:0 auto;width:stretch;height:10px"></canvas></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0))];
    abspos_auto_margins_width_stretch:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;inset:0;width:stretch;height:100px"></div></div>"# =>
        [("a", (8.0, 58.0, 200.0, 100.0))];
    abspos_auto_margins_max_width_stretch:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;inset:0;max-width:stretch;height:100px"></div></div>"# =>
        [("a", (8.0, 58.0, 200.0, 100.0))];
    abspos_auto_margins_height_stretch:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;inset:0;height:stretch;width:100px"></div></div>"# =>
        [("a", (58.0, 8.0, 100.0, 200.0))];
    abspos_auto_margins_max_height_stretch:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;inset:0;max-height:stretch;width:100px"></div></div>"# =>
        [("a", (58.0, 8.0, 100.0, 200.0))];
    abspos_auto_margins_width_stretch_between_insets:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;top:0;bottom:0;left:25px;right:25px;width:stretch"></div></div>"# =>
        [("a", (33.0, 8.0, 150.0, 200.0))];
    abspos_auto_margins_height_stretch_between_insets:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;top:25px;bottom:25px;left:0;right:0;height:stretch"></div></div>"# =>
        [("a", (8.0, 33.0, 200.0, 150.0))];
    abspos_auto_margins_width_stretch_without_insets:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;width:stretch;height:50px"></div></div>"# =>
        [("a", (8.0, 8.0, 200.0, 50.0))];
    abspos_auto_margins_width_stretch_with_left_inset:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;left:20px;width:stretch;height:50px"></div></div>"# =>
        [("a", (28.0, 8.0, 180.0, 50.0))];
    abspos_auto_margins_width_stretch_with_right_inset:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;right:20px;width:stretch;height:50px"></div></div>"# =>
        [("a", (8.0, 8.0, 180.0, 50.0))];
    abspos_lengths_margins_width_stretch:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:0 10px 0 5px;inset:0;width:stretch;height:50px"></div></div>"# =>
        [("a", (13.0, 8.0, 185.0, 50.0))];
    abspos_auto_margins_min_width_stretch:
        r#"<div style="position:relative;width:200px;height:200px"><div id=a style="position:absolute;margin:auto;inset:0;min-width:stretch;width:50px;height:50px"></div></div>"# =>
        [("a", (8.0, 83.0, 200.0, 50.0))];
    abspos_stretch_fills_from_the_static_position:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div></div>"# =>
        [("t", (58.0, 58.0, 100.0, 100.0))];
    abspos_stretch_from_the_static_position_less_margins:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch;margin:5px"></div></div></div>"# =>
        [("t", (63.0, 63.0, 90.0, 90.0))];
    abspos_width_stretch_from_the_static_position:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:20px"></div></div></div>"# =>
        [("t", (58.0, 58.0, 100.0, 20.0))];
    abspos_height_stretch_from_the_static_position:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:20px;height:stretch"></div></div></div>"# =>
        [("t", (58.0, 58.0, 20.0, 100.0))];
    abspos_stretch_fills_toward_the_start_of_a_right_to_left_block:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px;direction:rtl"><div style="margin-right:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div></div>"# =>
        [("t", (8.0, 58.0, 100.0, 100.0))];
    abspos_stretch_from_the_static_position_with_padding_and_border:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch;padding:5px;border:3px solid"></div></div></div>"# =>
        [("t", (58.0, 58.0, 100.0, 100.0))];
    abspos_stretch_from_the_static_position_in_a_padded_containing_block:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px;padding:7px;border:4px solid"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div></div>"# =>
        [("t", (69.0, 69.0, 85.0, 85.0))];
    abspos_stretch_from_the_static_position_between_min_and_max:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch;min-width:120px;max-height:30px"></div></div></div>"# =>
        [("t", (58.0, 58.0, 120.0, 30.0))];
    abspos_stretch_from_the_start_of_a_flex_container:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="display:flex;width:100px;height:100px;margin:20px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div></div>"# =>
        [("t", (28.0, 28.0, 130.0, 130.0))];
    abspos_stretch_toward_the_end_of_a_flex_container_aligned_to_the_end:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="display:flex;justify-content:flex-end;align-items:flex-end;width:100px;height:100px;margin:20px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div></div>"# =>
        [("t", (8.0, 8.0, 120.0, 120.0))];
    abspos_replaced_stretch_from_the_static_position:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><canvas id=t width=50 height=25 style="position:absolute;width:stretch;height:stretch"></canvas></div></div>"# =>
        [("t", (58.0, 58.0, 100.0, 100.0))];
    abspos_stretch_from_the_static_position_after_a_line:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px">xx<div id=t style="position:absolute;width:stretch;height:stretch"></div>yy</div></div>"# =>
        [("t", (58.0, 68.0, 100.0, 90.0))];
    abspos_stretch_from_the_static_position_in_vertical_rl:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px;writing-mode:vertical-rl"><div style="margin-right:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div></div>"# =>
        [("t", (8.0, 58.0, 100.0, 100.0))];
    abspos_stretch_from_the_static_position_in_vertical_lr:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px;writing-mode:vertical-lr"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div></div>"# =>
        [("t", (58.0, 58.0, 100.0, 100.0))];
    abspos_stretch_from_the_static_position_clamped_by_max_size:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch;max-width:40px;max-height:40px"></div></div></div>"# =>
        [("t", (58.0, 58.0, 40.0, 40.0))];
    abspos_stretch_from_the_static_position_of_an_inline_box:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="padding-left:30px;padding-top:20px">ab<span id=t style="position:absolute;width:stretch;height:stretch"></span></div></div>"# =>
        [("t", (58.0, 28.0, 100.0, 130.0))];
    abspos_stretch_from_the_static_position_two_blocks_down:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:20px;margin-top:20px"><div style="margin-left:30px;margin-top:30px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div></div></div>"# =>
        [("t", (58.0, 38.0, 100.0, 120.0))];
    abspos_stretch_in_a_flex_containing_block:
        r#"<div style="display:flex;position:relative;width:150px;height:150px;padding:10px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div>"# =>
        [("t", (18.0, 18.0, 140.0, 140.0))];
    abspos_stretch_from_the_static_position_in_a_scroll_container:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px;overflow:auto"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div></div>"# =>
        [("t", (58.0, 58.0, 100.0, 100.0))];
    abspos_stretch_with_right_and_bottom_insets_and_a_static_offset:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch;right:10px;bottom:10px"></div></div></div>"# =>
        [("t", (8.0, 8.0, 140.0, 140.0))];
    abspos_stretch_with_left_and_top_insets_and_a_static_offset:
        r#"<div style="display:flow-root;position:relative;width:150px;height:150px"><div style="margin-left:50px;margin-top:50px"><div id=t style="position:absolute;width:stretch;height:stretch;left:10px;top:10px"></div></div></div>"# =>
        [("t", (18.0, 18.0, 140.0, 140.0))];
    abspos_stretch_centered_in_a_flex_containing_block:
        r#"<div style="display:flex;justify-content:center;align-items:center;position:relative;width:150px;height:150px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div>"# =>
        [("t", (8.0, 8.0, 150.0, 150.0))];
    abspos_stretch_centered_in_the_inline_axis_of_a_flex_containing_block:
        r#"<div style="display:flex;justify-content:center;position:relative;width:150px;height:150px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div>"# =>
        [("t", (8.0, 8.0, 150.0, 150.0))];
    abspos_stretch_centered_in_the_block_axis_of_a_flex_containing_block:
        r#"<div style="display:flex;align-items:center;position:relative;width:150px;height:150px"><div id=t style="position:absolute;width:stretch;height:stretch"></div></div>"# =>
        [("t", (8.0, 8.0, 150.0, 150.0))];
    abspos_stretch_centered_in_a_flex_containing_block_less_margins:
        r#"<div style="display:flex;justify-content:center;align-items:center;position:relative;width:150px;height:150px"><div id=t style="position:absolute;width:stretch;height:stretch;margin:5px"></div></div>"# =>
        [("t", (13.0, 13.0, 140.0, 140.0))];
}
