//! `safe` and `unsafe` before the position of `justify-content` and
//! `align-content` (CSS Box Alignment 3 §4.4). Content that overflows its
//! alignment container is aligned as asked under `unsafe`, and falls back to
//! `start` under `safe`. A flex or grid container with neither keyword
//! overflows on both sides, while a block container keeps its content at the
//! start as `safe` does, whatever its `overflow` and writing mode. A `safe`
//! position the content fits in is aligned as written, and `place-content` sets
//! both longhands. The floats of a block container move with its content.
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
    flex_row_center_overflow:
        r#"<div style="display:flex;justify-content:center;width:50px;height:20px"><div id=t style="width:80px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (-7.0, 8.0, 80.0, 10.0))];
    flex_row_safe_center_overflow:
        r#"<div style="display:flex;justify-content:safe center;width:50px;height:20px"><div id=t style="width:80px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 8.0, 80.0, 10.0))];
    flex_row_unsafe_center_overflow:
        r#"<div style="display:flex;justify-content:unsafe center;width:50px;height:20px"><div id=t style="width:80px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (-7.0, 8.0, 80.0, 10.0))];
    flex_row_safe_center_fits:
        r#"<div style="display:flex;justify-content:safe center;width:100px;height:20px"><div id=t style="width:40px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (38.0, 8.0, 40.0, 10.0))];
    flex_row_safe_end_overflow:
        r#"<div style="display:flex;justify-content:safe end;width:50px;height:20px"><div id=t style="width:80px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 8.0, 80.0, 10.0))];
    flex_row_end_overflow:
        r#"<div style="display:flex;justify-content:end;width:50px;height:20px"><div id=t style="width:80px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (-22.0, 8.0, 80.0, 10.0))];
    flex_row_safe_flex_end_overflow:
        r#"<div style="display:flex;justify-content:safe flex-end;width:50px;height:20px"><div id=t style="width:80px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 8.0, 80.0, 10.0))];
    flex_row_reverse_safe_flex_start_overflow:
        r#"<div style="display:flex;flex-direction:row-reverse;justify-content:safe flex-start;width:50px;height:20px"><div id=t style="width:80px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 8.0, 80.0, 10.0))];
    flex_row_reverse_flex_start_overflow:
        r#"<div style="display:flex;flex-direction:row-reverse;justify-content:flex-start;width:50px;height:20px"><div id=t style="width:80px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (-22.0, 8.0, 80.0, 10.0))];
    flex_column_safe_center_overflow:
        r#"<div style="margin-top:100px;display:flex;flex-direction:column;justify-content:safe center;width:20px;height:50px"><div id=t style="width:10px;height:80px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 100.0, 10.0, 80.0))];
    flex_column_center_overflow:
        r#"<div style="margin-top:100px;display:flex;flex-direction:column;justify-content:center;width:20px;height:50px"><div id=t style="width:10px;height:80px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 85.0, 10.0, 80.0))];
    flex_wrap_align_content_safe_center_overflow:
        r#"<div style="display:flex;flex-wrap:wrap;align-content:safe center;width:50px;height:30px"><div  style="width:50px;height:20px;flex-shrink:0"></div><div id=t style="width:50px;height:20px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 28.0, 50.0, 20.0))];
    flex_wrap_align_content_center_overflow:
        r#"<div style="display:flex;flex-wrap:wrap;align-content:center;width:50px;height:30px"><div  style="width:50px;height:20px;flex-shrink:0"></div><div id=t style="width:50px;height:20px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 23.0, 50.0, 20.0))];
    flex_wrap_align_content_unsafe_end_overflow:
        r#"<div style="display:flex;flex-wrap:wrap;align-content:unsafe end;width:50px;height:30px"><div  style="width:50px;height:20px;flex-shrink:0"></div><div id=t style="width:50px;height:20px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 18.0, 50.0, 20.0))];
    flex_wrap_align_content_safe_end_overflow:
        r#"<div style="display:flex;flex-wrap:wrap;align-content:safe end;width:50px;height:30px"><div  style="width:50px;height:20px;flex-shrink:0"></div><div id=t style="width:50px;height:20px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 28.0, 50.0, 20.0))];
    grid_justify_content_center_overflow:
        r#"<div style="display:grid;grid-template-columns:80px;justify-content:center;width:50px;height:20px"><div id=t style="height:10px"></div></div>"# =>
        [("t", (-7.0, 8.0, 80.0, 10.0))];
    grid_justify_content_safe_center_overflow:
        r#"<div style="display:grid;grid-template-columns:80px;justify-content:safe center;width:50px;height:20px"><div id=t style="height:10px"></div></div>"# =>
        [("t", (8.0, 8.0, 80.0, 10.0))];
    grid_justify_content_unsafe_center_overflow:
        r#"<div style="display:grid;grid-template-columns:80px;justify-content:unsafe center;width:50px;height:20px"><div id=t style="height:10px"></div></div>"# =>
        [("t", (-7.0, 8.0, 80.0, 10.0))];
    grid_align_content_safe_end_overflow:
        r#"<div style="margin-top:100px;display:grid;grid-template-rows:80px;align-content:safe end;width:20px;height:50px"><div id=t></div></div>"# =>
        [("t", (8.0, 100.0, 20.0, 80.0))];
    grid_align_content_end_overflow:
        r#"<div style="margin-top:100px;display:grid;grid-template-rows:80px;align-content:end;width:20px;height:50px"><div id=t></div></div>"# =>
        [("t", (8.0, 70.0, 20.0, 80.0))];
    block_align_content_safe_center_overflow:
        r#"<div style="margin-top:100px;align-content:safe center;width:50px;height:20px"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 100.0, 50.0, 80.0))];
    block_align_content_unsafe_center_overflow:
        r#"<div style="margin-top:100px;align-content:unsafe center;width:50px;height:20px"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 70.0, 50.0, 80.0))];
    block_align_content_safe_end_overflow:
        r#"<div style="margin-top:100px;align-content:safe end;width:50px;height:20px"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 100.0, 50.0, 80.0))];
    block_align_content_unsafe_end_overflow:
        r#"<div style="margin-top:100px;align-content:unsafe end;width:50px;height:20px"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 40.0, 50.0, 80.0))];
    block_align_content_safe_center_fits:
        r#"<div style="margin-top:100px;align-content:safe center;width:50px;height:100px"><div id=t style="height:20px"></div></div>"# =>
        [("t", (8.0, 140.0, 50.0, 20.0))];
    flex_place_content_safe_center_overflow:
        r#"<div style="display:flex;place-content:safe center;width:50px;height:20px"><div id=t style="width:80px;height:10px;flex-shrink:0"></div></div>"# =>
        [("t", (8.0, 8.0, 80.0, 10.0))];
    block_plain_center:
        r#"<div style="margin-top:100px;align-content:center;width:50px;height:20px"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 100.0, 50.0, 80.0))];
    block_plain_end:
        r#"<div style="margin-top:100px;align-content:end;width:50px;height:20px"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 100.0, 50.0, 80.0))];
    block_plain_center_overflow_auto:
        r#"<div style="margin-top:100px;align-content:center;width:50px;height:20px;overflow:auto"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 100.0, 50.0, 80.0))];
    block_plain_center_overflow_hidden:
        r#"<div style="margin-top:100px;align-content:center;width:50px;height:20px;overflow:hidden"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 100.0, 50.0, 80.0))];
    block_plain_center_flow_root:
        r#"<div style="margin-top:100px;align-content:center;width:50px;height:20px;display:flow-root"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 100.0, 50.0, 80.0))];
    block_plain_center_fits:
        r#"<div style="margin-top:100px;align-content:center;width:50px;height:120px"><div id=t style="height:20px"></div></div>"# =>
        [("t", (8.0, 150.0, 50.0, 20.0))];
    block_plain_center_float:
        r#"<div style="margin-top:100px;align-content:center;width:50px;height:20px;display:flow-root"><div id=t style="float:left;width:10px;height:80px"></div></div>"# =>
        [("t", (8.0, 100.0, 10.0, 80.0))];
    block_plain_flex_end_overflow:
        r#"<div style="margin-top:100px;align-content:flex-end;width:50px;height:20px"><div id=t style="height:80px"></div></div>"# =>
        [("t", (8.0, 100.0, 50.0, 80.0))];
    block_plain_center_vertical_rl:
        r#"<div style="margin-top:100px;align-content:center;writing-mode:vertical-rl;width:20px;height:50px"><div id=t style="width:80px"></div></div>"# =>
        [("t", (-52.0, 100.0, 80.0, 50.0))];
    block_unsafe_center_vertical_rl:
        r#"<div style="margin-top:100px;align-content:unsafe center;writing-mode:vertical-rl;width:20px;height:50px"><div id=t style="width:80px"></div></div>"# =>
        [("t", (-22.0, 100.0, 80.0, 50.0))];
    block_plain_center_abs_child:
        r#"<div style="margin-top:100px;align-content:center;width:50px;height:20px;position:relative"><div id=t style="position:absolute;width:10px;height:80px"></div></div>"# =>
        [("t", (8.0, 110.0, 10.0, 80.0))];
    block_center_moves_a_float_with_the_content:
        r#"<div style="display:flow-root;align-content:center;height:100px;width:50px"><div id=f style="float:left;width:10px;height:20px"></div><div id=t style="height:20px"></div></div>"# =>
        [("f", (8.0, 48.0, 10.0, 20.0)), ("t", (8.0, 48.0, 50.0, 20.0))];
    block_center_moves_a_float_alone:
        r#"<div style="display:flow-root;align-content:center;height:100px;width:50px"><div id=t style="float:left;width:10px;height:20px"></div></div>"# =>
        [("t", (8.0, 48.0, 10.0, 20.0))];
    block_center_centers_the_taller_float_with_the_content:
        r#"<div style="display:flow-root;align-content:center;height:100px;width:50px"><div id=t style="float:left;width:10px;height:40px"></div><div style="height:20px"></div></div>"# =>
        [("t", (8.0, 38.0, 10.0, 40.0))];
    block_end_moves_a_right_float:
        r#"<div style="display:flow-root;align-content:end;height:100px;width:50px"><div id=t style="float:right;width:10px;height:20px"></div></div>"# =>
        [("t", (48.0, 88.0, 10.0, 20.0))];
    block_unsafe_center_moves_an_overflowing_float:
        r#"<div style="margin-top:100px;display:flow-root;align-content:unsafe center;height:20px;width:50px"><div id=t style="float:left;width:10px;height:60px"></div></div>"# =>
        [("t", (8.0, 80.0, 10.0, 60.0))];
    block_center_moves_floats_of_a_plain_block_and_contains_them:
        r#"<div style="align-content:center;height:100px;width:50px"><div id=t style="float:left;width:10px;height:40px"></div><div id=n style="height:20px"></div></div><div id=s style="height:10px"></div>"# =>
        [("t", (8.0, 38.0, 10.0, 40.0)), ("n", (8.0, 38.0, 50.0, 20.0)), ("s", (8.0, 108.0, 784.0, 10.0))];
    block_center_keeps_a_float_taller_than_the_block_at_the_start:
        r#"<div style="align-content:center;height:30px;width:50px"><div id=t style="float:left;width:10px;height:40px"></div><div id=n style="height:20px"></div></div><div id=s style="height:10px"></div>"# =>
        [("t", (8.0, 8.0, 10.0, 40.0)), ("n", (8.0, 8.0, 50.0, 20.0)), ("s", (8.0, 38.0, 784.0, 10.0))];
    block_unsafe_center_moves_floats_of_a_plain_block_up:
        r#"<div style="margin-top:100px;align-content:unsafe center;height:30px;width:50px"><div id=t style="float:left;width:10px;height:40px"></div><div id=n style="height:20px"></div></div><div id=s style="height:10px"></div>"# =>
        [("t", (8.0, 95.0, 10.0, 40.0)), ("n", (8.0, 95.0, 50.0, 20.0)), ("s", (8.0, 130.0, 784.0, 10.0))];
}
