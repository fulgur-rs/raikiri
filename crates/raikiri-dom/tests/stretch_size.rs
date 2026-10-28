//! `stretch` (and its alias `-webkit-fill-available`) as the size of a box
//! (CSS Sizing 4 §5.1): the margin box fills the containing block. The inline
//! axis subtracts the margins; the block axis needs a containing block whose
//! height is definite, takes a margin off only where the parent holds it in
//! (a box that roots its own formatting context, or one with a border or
//! padding on that side), and otherwise behaves as `auto`.
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
