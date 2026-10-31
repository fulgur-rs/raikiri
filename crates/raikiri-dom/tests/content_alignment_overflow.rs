//! `safe` and `unsafe` before the position of `justify-content` and
//! `align-content` (CSS Box Alignment 3 §4.4). Content that overflows its
//! alignment container is aligned as asked under `unsafe`, and falls back to
//! `start` under `safe`. A flex or grid container with neither keyword
//! overflows on both sides. A `safe` position the content fits in is aligned as
//! written, and `place-content` sets both longhands.
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
}
