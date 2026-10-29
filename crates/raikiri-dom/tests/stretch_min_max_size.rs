//! `stretch` (and its alias `-webkit-fill-available`) as the `min-width`,
//! `max-width`, `min-height` or `max-height` of a box (CSS Sizing 4 §5.1): it
//! is the size `width: stretch` or `height: stretch` would give the box. The
//! inline axis fills the width of the containing block less the margins of the
//! box, or the room a float leaves a box that keeps clear of it. The block axis
//! needs a containing block whose height is definite and takes a margin off only
//! where the parent holds it in; an indefinite room makes a minimum `0` and a
//! maximum `none`, and the intrinsic sizes of a parent leave the keyword out.
//!
//! The expected border boxes are the ones Chromium computes for the same markup
//! in an 800px by 600px viewport, with no body margin and `10px Ahem` text.

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
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }}          html, body {{ margin: 0 }} body {{ font: 10px/10px Ahem }}          div {{ box-sizing: border-box }}</style>{body}"
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
    width_min_stretch_on_a_block:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_min_webkit_fill_available:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:50px;min-width:-webkit-fill-available"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_min_stretch_logical:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:50px;min-inline-size:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_max_stretch_cuts_a_wide_block:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:300px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_max_stretch_keeps_a_narrow_block:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:100px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 100.0, 20.0))];
    width_max_stretch_on_an_auto_width:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_min_stretch_raises_a_block_wider_than_the_room:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:300px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 300.0, 20.0))];
    width_min_stretch_takes_the_margins_off:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;margin:0 10px;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (10.0, 0.0, 180.0, 20.0))];
    width_max_stretch_takes_the_margins_off:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;margin:0 10px 0 30px;width:300px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (30.0, 0.0, 160.0, 20.0))];
    width_min_stretch_with_auto_margins:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;margin:0 auto;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_min_stretch_is_a_border_box_size:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;padding:0 10px;border:5px solid;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_max_stretch_is_a_border_box_size:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;padding:0 10px;border:5px solid;width:300px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_min_stretch_of_a_border_box:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;box-sizing:border-box;padding:0 10px;border:5px solid;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_min_stretch_wins_over_a_smaller_max:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:50px;min-width:stretch;max-width:120px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_min_length_wins_over_max_stretch:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:50px;max-width:stretch;min-width:250px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 250.0, 20.0))];
    width_min_and_max_stretch:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:50px;min-width:stretch;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_later_length_replaces_min_stretch:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:50px;min-width:stretch;min-width:20px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 50.0, 20.0))];
    width_a_later_stretch_replaces_min_length:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;width:50px;min-width:20px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_min_stretch_of_a_float:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;float:left;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 0.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_min_stretch_of_a_float_with_margins:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;float:left;margin:0 10px;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 0.0)), ("c", (10.0, 0.0, 180.0, 20.0))];
    width_a_max_stretch_of_a_float:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;float:left;width:300px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 0.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_an_auto_float_with_max_stretch_shrinks:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;float:left;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 0.0)), ("c", (0.0, 0.0, 0.0, 20.0))];
    width_a_min_stretch_of_an_inline_block:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;display:inline-block;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 22.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_max_stretch_of_an_inline_block:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;display:inline-block;width:300px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 22.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_min_stretch_of_an_inline_block_with_margins:
        r#"<div id=o style="width:200px;"><div id=c style="height:20px;display:inline-block;margin:0 10px;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 22.0)), ("c", (10.0, 0.0, 180.0, 20.0))];
    width_a_min_stretch_of_a_flex_item:
        r#"<div id=o style="width:200px;display:flex"><div id=c style="height:20px;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_max_stretch_of_a_flex_item:
        r#"<div id=o style="width:200px;display:flex"><div id=c style="height:20px;width:300px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_min_stretch_of_a_column_flex_item:
        r#"<div id=o style="width:200px;display:flex;flex-direction:column"><div id=c style="height:20px;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_max_stretch_of_a_column_flex_item:
        r#"<div id=o style="width:200px;display:flex;flex-direction:column"><div id=c style="height:20px;width:300px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_min_stretch_of_a_flex_item_beside_another:
        r#"<div id=o style="width:200px;display:flex"><div id=c style="height:20px;width:50px;min-width:stretch"></div><div id=d style="width:60px;height:20px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0)), ("d", (200.0, 0.0, 0.0, 20.0))];
    width_a_min_stretch_of_a_flex_item_with_margins:
        r#"<div id=o style="width:200px;display:flex"><div id=c style="height:20px;margin:0 10px;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (10.0, 0.0, 180.0, 20.0))];
    width_a_min_stretch_of_a_grid_item:
        r#"<div id=o style="width:200px;display:grid"><div id=c style="height:20px;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 200.0, 20.0))];
    width_a_max_stretch_of_a_grid_item:
        r#"<div id=o style="width:200px;display:grid"><div id=c style="height:20px;width:300px;max-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 300.0, 20.0))];
    width_a_min_stretch_of_a_grid_item_in_a_track:
        r#"<div id=o style="width:200px;display:grid;grid-template-columns:100px"><div id=c style="height:20px;width:50px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 100.0, 20.0))];
    width_a_min_stretch_of_a_shrink_wrapped_float_child:
        r#"<div id=o style="float:left"><div id=c style="height:20px;min-width:stretch;width:50px"></div><div style="width:120px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 120.0, 30.0)), ("c", (0.0, 0.0, 120.0, 20.0))];
    width_a_max_stretch_of_a_shrink_wrapped_inline_block_child:
        r#"<div id=o style="display:inline-block"><div id=c style="height:20px;max-width:stretch;width:300px"></div><div style="width:120px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 300.0, 30.0)), ("c", (0.0, 0.0, 300.0, 20.0))];
    width_a_min_stretch_alone_in_a_shrink_wrapped_float:
        r#"<div id=o style="float:left"><div id=c style="height:20px;min-width:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 0.0, 20.0)), ("c", (0.0, 0.0, 0.0, 20.0))];
    float_a_min_stretch_fills_the_room_beside_a_float:
        r#"<div id=o style="width:200px;"><div style="float:left;width:100px;height:75px"></div><div id=c style="height:20px;display:flow-root;width:0;min-width:stretch;height:25px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (100.0, 0.0, 100.0, 25.0))];
    float_a_max_stretch_fills_the_room_beside_a_float:
        r#"<div id=o style="width:200px;"><div style="float:left;width:100px;height:75px"></div><div id=c style="height:20px;display:flow-root;width:1000px;max-width:stretch;height:25px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (100.0, 0.0, 100.0, 25.0))];
    float_a_stretch_width_with_min_stretch:
        r#"<div id=o style="width:200px;"><div style="float:left;width:100px;height:75px"></div><div id=c style="height:20px;display:flow-root;width:stretch;min-width:stretch;height:25px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (100.0, 0.0, 100.0, 25.0))];
    float_a_min_stretch_with_a_margin_beside_a_float:
        r#"<div id=o style="width:200px;"><div style="float:left;width:100px;height:75px"></div><div id=c style="height:20px;display:flow-root;width:0;min-width:stretch;margin-left:10px;height:25px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (100.0, 0.0, 100.0, 25.0))];
    float_a_min_stretch_of_a_plain_block_is_not_narrowed:
        r#"<div id=o style="width:200px;"><div style="float:left;width:100px;height:75px"></div><div id=c style="height:20px;width:0;min-width:stretch;height:25px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (0.0, 0.0, 200.0, 25.0))];
    float_a_min_stretch_drops_a_box_below_floats_that_leave_no_room:
        r#"<div id=o style="width:200px;"><div style="float:left;width:150px;height:10px"></div><div id=c style="height:20px;display:flow-root;width:0;min-width:stretch;height:25px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (150.0, 0.0, 50.0, 25.0))];
    float_a_min_stretch_of_a_canvas:
        r#"<div id=o style="width:200px;"><div style="float:left;width:100px;height:75px"></div><canvas id=c style="display:block;width:0;min-width:stretch;height:25px"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (100.0, 0.0, 100.0, 25.0))];
    float_a_max_stretch_of_a_canvas:
        r#"<div id=o style="width:200px;"><div style="float:left;width:100px;height:75px"></div><canvas id=c style="display:block;width:1000px;max-width:stretch;height:25px"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (100.0, 0.0, 100.0, 25.0))];
    float_a_min_stretch_of_a_canvas_with_auto_height:
        r#"<div id=o style="width:200px;"><div style="float:left;width:100px;height:75px"></div><canvas id=c width=100 height=50 style="display:block;min-width:stretch"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 50.0)), ("c", (100.0, 0.0, 100.0, 50.0))];
    width_a_max_stretch_of_a_canvas:
        r#"<div id=o style="width:200px;"><canvas id=c style="display:block;width:1000px;max-width:stretch;height:25px"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (0.0, 0.0, 200.0, 25.0))];
    width_a_min_stretch_of_an_image:
        r#"<div id=o style="width:200px;"><img id=c style="display:block;width:10px;min-width:stretch;height:25px" src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"></div>"# =>
        [("o", (0.0, 0.0, 200.0, 25.0)), ("c", (0.0, 0.0, 200.0, 25.0))];
    height_min_stretch_on_a_block:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_webkit_fill_available:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;min-height:-webkit-fill-available"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_logical:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;min-block-size:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_raises_a_short_block:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;height:30px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_max_stretch_cuts_a_tall_block:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;height:500px;max-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_max_stretch_keeps_a_short_block:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;height:30px;max-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 30.0))];
    height_max_stretch_on_an_auto_height:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;max-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 0.0))];
    height_min_stretch_with_content_taller_than_the_room:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;min-height:stretch"><div style="height:150px"></div></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 150.0))];
    height_min_stretch_keeps_a_margin_the_parent_does_not_hold:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;min-height:stretch;margin-bottom:50px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_keeps_a_top_margin:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;min-height:stretch;margin-top:50px"></div></div>"# =>
        [("o", (0.0, 50.0, 200.0, 100.0)), ("c", (0.0, 50.0, 50.0, 100.0))];
    height_min_stretch_keeps_both_margins:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;min-height:stretch;margin:20px 0 30px"></div></div>"# =>
        [("o", (0.0, 20.0, 200.0, 100.0)), ("c", (0.0, 20.0, 50.0, 100.0))];
    height_min_stretch_takes_the_margin_a_flow_root_parent_holds:
        r#"<div id=o style="width:200px;height:100px;display:flow-root"><div id=c style="width:50px;min-height:stretch;margin-bottom:50px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 50.0))];
    height_min_stretch_takes_both_margins_a_flow_root_parent_holds:
        r#"<div id=o style="width:200px;height:100px;display:flow-root"><div id=c style="width:50px;min-height:stretch;margin:20px 0 30px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 20.0, 50.0, 50.0))];
    height_min_stretch_takes_the_margin_under_a_padded_parent:
        r#"<div id=o style="width:200px;height:100px;padding-bottom:1px"><div id=c style="width:50px;min-height:stretch;margin-bottom:50px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 49.0))];
    height_min_stretch_takes_the_margin_under_a_bordered_parent:
        r#"<div id=o style="width:200px;height:100px;border-bottom:1px solid"><div id=c style="width:50px;min-height:stretch;margin-bottom:50px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 49.0))];
    height_max_stretch_takes_the_margin_a_flow_root_parent_holds:
        r#"<div id=o style="width:200px;height:100px;display:flow-root"><div id=c style="width:50px;height:500px;max-height:stretch;margin-bottom:50px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 50.0))];
    height_max_stretch_keeps_a_margin_the_parent_does_not_hold:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;height:500px;max-height:stretch;margin-bottom:50px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_is_a_border_box_size:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;padding:10px;border:5px solid;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_max_stretch_is_a_border_box_size:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;padding:10px;border:5px solid;height:500px;max-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_under_a_padded_parent:
        r#"<div id=o style="width:200px;height:100px;padding:10px;box-sizing:content-box"><div id=c style="width:50px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 220.0, 120.0)), ("c", (10.0, 10.0, 50.0, 100.0))];
    height_min_stretch_under_a_border_box_parent:
        r#"<div id=o style="width:200px;height:100px;box-sizing:border-box;padding:10px"><div id=c style="width:50px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (10.0, 10.0, 50.0, 80.0))];
    height_min_stretch_of_an_indefinite_parent:
        r#"<div id=o style="width:200px;"><div id=c style="width:50px;height:30px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 30.0)), ("c", (0.0, 0.0, 50.0, 30.0))];
    height_max_stretch_of_an_indefinite_parent:
        r#"<div id=o style="width:200px;"><div id=c style="width:50px;height:300px;max-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 300.0)), ("c", (0.0, 0.0, 50.0, 300.0))];
    height_min_stretch_of_an_indefinite_parent_with_no_height:
        r#"<div id=o style="width:200px;"><div id=c style="width:50px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 0.0)), ("c", (0.0, 0.0, 50.0, 0.0))];
    height_min_stretch_of_an_indefinite_parent_with_content:
        r#"<div id=o style="width:200px;"><div id=c style="width:50px;min-height:stretch"><div style="height:10px"></div></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 10.0)), ("c", (0.0, 0.0, 50.0, 10.0))];
    height_min_stretch_of_a_parent_with_only_a_min_height:
        r#"<div id=o style="width:200px;min-height:100px"><div id=c style="width:50px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 0.0))];
    height_max_stretch_of_a_parent_with_only_a_max_height:
        r#"<div id=o style="width:200px;max-height:100px"><div id=c style="width:50px;height:300px;max-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 300.0))];
    height_min_stretch_of_a_parent_with_a_percentage_height:
        r#"<div style="height:400px"><div id=o style="width:200px;height:50%"><div id=c style="width:50px;min-height:stretch"></div></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 200.0)), ("c", (0.0, 0.0, 50.0, 200.0))];
    height_min_stretch_of_a_float:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;float:left;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_of_a_float_with_margins:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;float:left;min-height:stretch;margin:10px 0"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 10.0, 50.0, 80.0))];
    height_max_stretch_of_a_float:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;float:left;height:500px;max-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_of_an_inline_block:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;display:inline-block;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_of_an_inline_block_with_margins:
        r#"<div id=o style="width:200px;height:100px;"><div id=c style="width:50px;display:inline-block;min-height:stretch;margin:10px 0"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 10.0, 50.0, 80.0))];
    height_min_stretch_of_a_flex_item:
        r#"<div id=o style="width:200px;height:100px;display:flex;align-items:flex-start"><div id=c style="width:50px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_max_stretch_of_a_flex_item:
        r#"<div id=o style="width:200px;height:100px;display:flex;align-items:flex-start"><div id=c style="width:50px;height:500px;max-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_of_a_flex_item_with_margins:
        r#"<div id=o style="width:200px;height:100px;display:flex;align-items:flex-start"><div id=c style="width:50px;min-height:stretch;margin:10px 0"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 10.0, 50.0, 80.0))];
    height_min_stretch_of_a_column_flex_item:
        r#"<div id=o style="width:200px;height:100px;display:flex;flex-direction:column"><div id=c style="width:50px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_max_stretch_of_a_column_flex_item:
        r#"<div id=o style="width:200px;height:100px;display:flex;flex-direction:column"><div id=c style="width:50px;height:500px;max-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_of_a_grid_item:
        r#"<div id=o style="width:200px;height:100px;display:grid;align-items:start"><div id=c style="width:50px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    height_min_stretch_of_a_grid_item_with_margins:
        r#"<div id=o style="width:200px;height:100px;display:grid;align-items:start"><div id=c style="width:50px;min-height:stretch;margin:10px 0"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 10.0, 50.0, 80.0))];
    height_min_stretch_of_a_grid_item_in_a_track:
        r#"<div id=o style="width:200px;height:100px;display:grid;align-items:start;grid-template-rows:60px"><div id=c style="width:50px;min-height:stretch"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 60.0))];
    height_min_stretch_through_a_percentage_chain:
        r#"<div id=o style="height:100px"><div style="height:100%"><div id=c style="width:50px;min-height:stretch"></div></div></div>"# =>
        [("o", (0.0, 0.0, 800.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    abspos_min_height_below_a_top_inset:
        r#"<div id=o style="position:relative;width:50px;height:40px;padding:3px 5px;margin:5px"><div id=c style="position:absolute;top:10px;width:10px;min-height:stretch;margin:1px 0 2px"></div></div>"# =>
        [("o", (5.0, 5.0, 50.0, 40.0)), ("c", (10.0, 16.0, 10.0, 27.0))];
    abspos_min_height_above_a_bottom_inset:
        r#"<div id=o style="position:relative;width:50px;height:40px;padding:3px 5px;margin:5px"><div id=c style="position:absolute;bottom:10px;width:10px;min-height:stretch;margin:1px 0 2px"></div></div>"# =>
        [("o", (5.0, 5.0, 50.0, 40.0)), ("c", (10.0, 6.0, 10.0, 27.0))];
    abspos_min_height_from_the_static_position:
        r#"<div id=o style="position:relative;width:50px;height:40px;padding:3px 5px;margin:5px"><div id=c style="position:absolute;width:10px;min-height:stretch;"></div></div>"# =>
        [("o", (5.0, 5.0, 50.0, 40.0)), ("c", (10.0, 8.0, 10.0, 37.0))];
    abspos_min_height_between_zero_insets:
        r#"<div id=o style="position:relative;width:50px;height:40px;padding:3px 5px;margin:5px"><div id=c style="position:absolute;top:0;bottom:0;width:10px;min-height:stretch;"></div></div>"# =>
        [("o", (5.0, 5.0, 50.0, 40.0)), ("c", (10.0, 5.0, 10.0, 40.0))];
    abspos_min_width_beside_a_left_inset:
        r#"<div id=o style="position:relative;width:50px;height:40px;padding:3px 5px;margin:5px"><div id=c style="position:absolute;left:10px;height:10px;min-width:stretch;margin:0 1px 0 2px"></div></div>"# =>
        [("o", (5.0, 5.0, 50.0, 40.0)), ("c", (17.0, 8.0, 37.0, 10.0))];
    abspos_min_width_from_the_static_position:
        r#"<div id=o style="position:relative;width:50px;height:40px;padding:3px 5px;margin:5px"><div id=c style="position:absolute;height:10px;min-width:stretch;"></div></div>"# =>
        [("o", (5.0, 5.0, 50.0, 40.0)), ("c", (10.0, 8.0, 45.0, 10.0))];
    abspos_max_height_below_a_top_inset:
        r#"<div id=o style="position:relative;width:50px;height:40px;padding:3px 5px;margin:5px"><div id=c style="position:absolute;top:10px;width:10px;height:500px;max-height:stretch;margin:1px 0 2px"></div></div>"# =>
        [("o", (5.0, 5.0, 50.0, 40.0)), ("c", (10.0, 16.0, 10.0, 27.0))];
    abspos_max_width_beside_a_left_inset:
        r#"<div id=o style="position:relative;width:50px;height:40px;padding:3px 5px;margin:5px"><div id=c style="position:absolute;left:10px;height:10px;width:500px;max-width:stretch;margin:0 1px 0 2px"></div></div>"# =>
        [("o", (5.0, 5.0, 50.0, 40.0)), ("c", (17.0, 8.0, 37.0, 10.0))];
    abspos_min_height_below_an_inset_past_the_edge:
        r#"<div id=o style="position:relative;width:50px;height:40px;padding:3px 5px;margin:5px"><div id=c style="position:absolute;top:60px;width:10px;min-height:stretch;"></div></div>"# =>
        [("o", (5.0, 5.0, 50.0, 40.0)), ("c", (10.0, 65.0, 10.0, 0.0))];
    replaced_height_a_min_stretch_of_a_canvas:
        r#"<div id=o style="width:200px;height:100px;"><canvas id=c width=40 height=20 style="display:block;width:50px;height:20px;min-height:stretch"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    replaced_height_a_max_stretch_of_a_canvas:
        r#"<div id=o style="width:200px;height:100px;"><canvas id=c width=40 height=20 style="display:block;width:50px;height:500px;max-height:stretch"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    replaced_height_a_min_stretch_of_an_image:
        r#"<div id=o style="width:200px;height:100px;"><img id=c src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7" style="display:block;width:50px;height:20px;min-height:stretch"></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    replaced_height_a_max_stretch_of_an_image:
        r#"<div id=o style="width:200px;height:100px;"><img id=c src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7" style="display:block;width:50px;height:500px;max-height:stretch"></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    replaced_height_a_min_stretch_with_a_top_margin:
        r#"<div id=o style="width:200px;height:100px;"><canvas id=c width=40 height=20 style="display:block;width:50px;height:20px;min-height:stretch;margin-top:10px"></canvas></div>"# =>
        [("o", (0.0, 10.0, 200.0, 100.0)), ("c", (0.0, 10.0, 50.0, 100.0))];
    replaced_height_a_max_stretch_with_a_bottom_margin:
        r#"<div id=o style="width:200px;height:100px;"><canvas id=c width=40 height=20 style="display:block;width:50px;height:500px;max-height:stretch;margin-bottom:10px"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    replaced_height_a_min_stretch_in_an_auto_height_parent:
        r#"<div id=o style="width:200px"><canvas id=c width=40 height=20 style="display:block;width:50px;height:20px;min-height:stretch"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 20.0)), ("c", (0.0, 0.0, 50.0, 20.0))];
    replaced_height_a_max_stretch_in_an_auto_height_parent:
        r#"<div id=o style="width:200px"><canvas id=c width=40 height=20 style="display:block;width:50px;height:500px;max-height:stretch"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 500.0)), ("c", (0.0, 0.0, 50.0, 500.0))];
    replaced_height_both_stretch:
        r#"<div id=o style="width:200px;height:100px;"><canvas id=c width=40 height=20 style="display:block;width:50px;height:20px;min-height:stretch;max-height:stretch"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    replaced_height_a_min_stretch_of_an_inline_canvas:
        r#"<div id=o style="width:200px;height:100px;"><canvas id=c width=40 height=20 style="width:50px;height:20px;min-height:stretch"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    replaced_height_a_max_stretch_with_a_ratio:
        r#"<div id=o style="width:200px;height:100px;"><canvas id=c width=40 height=20 style="display:block;width:50px;height:auto;max-height:stretch;aspect-ratio:1/10"></canvas></div>"# =>
        [("o", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 50.0, 100.0))];
    replaced_width_a_stretch_in_a_shrink_to_fit_float:
        r#"<div id=o style="float:left"><canvas id=c width=40 height=20 style="display:block;width:50px;height:20px;min-width:stretch"></canvas></div>"# =>
        [("o", (0.0, 0.0, 50.0, 20.0)), ("c", (0.0, 0.0, 50.0, 20.0))];
    replaced_width_a_max_stretch_in_a_shrink_to_fit_float:
        r#"<div id=o style="float:left"><canvas id=c width=40 height=20 style="display:block;width:500px;height:20px;max-width:stretch"></canvas></div>"# =>
        [("o", (0.0, 0.0, 500.0, 20.0)), ("c", (0.0, 0.0, 500.0, 20.0))];
    width_a_min_stretch_in_a_shrink_to_fit_float:
        r#"<div id=o style="float:left"><div id=c style="width:30px;min-width:stretch;height:20px"></div></div>"# =>
        [("o", (0.0, 0.0, 30.0, 20.0)), ("c", (0.0, 0.0, 30.0, 20.0))];
    width_a_max_stretch_in_a_shrink_to_fit_float:
        r#"<div id=o style="float:left"><div id=c style="width:300px;max-width:stretch;height:20px"></div></div>"# =>
        [("o", (0.0, 0.0, 300.0, 20.0)), ("c", (0.0, 0.0, 300.0, 20.0))];
}
