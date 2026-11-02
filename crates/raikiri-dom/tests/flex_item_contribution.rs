//! The contribution of a flex item to the size of its container when that size
//! follows the items: the min-content or max-content width of a row container
//! (an inline-block, a float, an inline-flex container, a box that states a
//! `min-content`, `max-content` or `fit-content` width, a wrapping or reversed
//! container) and the height of a column container that has none of its own.
//!
//! An item whose `width` is a `min-content` or `max-content` keyword adds the
//! width the keyword stands for. A scroll container adds its content, within
//! the bounds its flex base size and its `min-width` and `max-width` give, not
//! its flex base size alone. An item of a column container whose flex base size
//! is a length is as tall as that size, within its minimum height, whatever
//! `height` it states; its content is measured at the width its `width`,
//! `min-width` and `max-width` give it.
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
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }} \
         td {{ padding: 0 }} table {{ border-spacing: 0 }}</style>{body}"
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
    an_inline_block_keeps_a_min_content_grid_item_with_a_zero_basis_and_padding_at_its_min_content_width:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:grid;width:min-content;padding:6px 2px;overflow:auto;font-size:2em;flex:1 1 0">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 164.0, 42.0)), ("g", (91.0, 11.0, 84.0, 42.0)), ("o", (8.0, 8.0, 170.0, 48.0))];
    an_inline_block_counts_a_min_content_grid_item_without_a_basis:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:grid;width:min-content;padding:6px 2px;overflow:auto;font-size:2em">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 164.0, 42.0)), ("g", (91.0, 11.0, 84.0, 42.0)), ("o", (8.0, 8.0, 170.0, 48.0))];
    an_inline_block_counts_a_min_content_grid_item_with_a_zero_basis_without_scrolling:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:grid;width:min-content;padding:6px 2px;font-size:2em;flex:1 1 0">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 164.0, 42.0)), ("g", (91.0, 11.0, 84.0, 42.0)), ("o", (8.0, 8.0, 170.0, 48.0))];
    an_inline_block_counts_a_min_content_block_item_with_a_zero_basis_and_scrolling:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:block;width:min-content;padding:6px 2px;overflow:auto;font-size:2em;flex:1 1 0">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 164.0, 42.0)), ("g", (91.0, 11.0, 84.0, 42.0)), ("o", (8.0, 8.0, 170.0, 48.0))];
    an_inline_block_counts_a_min_content_block_item_with_a_zero_basis:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:block;width:min-content;padding:6px 2px;font-size:2em;flex:1 1 0">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 164.0, 42.0)), ("g", (91.0, 11.0, 84.0, 42.0)), ("o", (8.0, 8.0, 170.0, 48.0))];
    an_inline_block_counts_a_block_item_with_a_zero_basis_at_its_max_content_width:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:block;padding:6px 2px;font-size:2em;flex:1 1 0">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 324.0, 22.0)), ("g", (91.0, 11.0, 244.0, 22.0)), ("o", (8.0, 8.0, 330.0, 28.0))];
    an_inline_block_counts_a_scrolling_block_item_with_a_zero_basis_at_its_max_content_width:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:block;padding:6px 2px;font-size:2em;flex:1 1 0;overflow:auto">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 324.0, 22.0)), ("g", (91.0, 11.0, 244.0, 22.0)), ("o", (8.0, 8.0, 330.0, 28.0))];
    a_min_content_item_with_a_zero_basis_counts_its_longest_word:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:block;width:min-content;flex:1 1 0">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 120.0, 30.0)), ("g", (91.0, 11.0, 40.0, 30.0)), ("o", (8.0, 8.0, 126.0, 36.0))];
    a_scrolling_min_content_item_with_a_zero_basis_counts_its_longest_word:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:block;width:min-content;flex:1 1 0;overflow:auto">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 120.0, 30.0)), ("g", (91.0, 11.0, 40.0, 30.0)), ("o", (8.0, 8.0, 126.0, 36.0))];
    an_item_with_a_20px_width_and_a_zero_basis_counts_its_width:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:block;width:20px;flex:1 1 0">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 100.0, 30.0)), ("g", (91.0, 11.0, 20.0, 30.0)), ("o", (8.0, 8.0, 106.0, 36.0))];
    a_max_content_item_with_a_zero_basis_counts_its_max_content_width:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:block;width:max-content;flex:1 1 0">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 200.0, 10.0)), ("g", (91.0, 11.0, 120.0, 10.0)), ("o", (8.0, 8.0, 206.0, 16.0))];
    a_fit_content_item_with_a_zero_basis_counts_its_max_content_width:
        r#"<span id=o style="display:inline-block;max-width:60%;border:3px double"><div id=f style="display:flex">jj kk bb <div id=g style="display:block;width:fit-content;flex:1 1 0">ccc ddd eeee </div></div></span>"# =>
        [("f", (11.0, 11.0, 200.0, 10.0)), ("g", (91.0, 11.0, 120.0, 10.0)), ("o", (8.0, 8.0, 206.0, 16.0))];
    row_scrolling_item_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_flex_1_0_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="flex:1 0 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_with_a_20px_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:20px;flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 20.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 50.0, 36.0))];
    row_scrolling_item_with_a_20px_width_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:20px;flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (32.23, 8.0, 25.77, 36.0)), ("b", (8.0, 8.0, 24.23, 36.0)), ("o", (8.0, 8.0, 50.0, 36.0))];
    row_scrolling_item_with_a_20px_width_flex_1_0_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:20px;flex:1 0 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 20.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 50.0, 36.0))];
    row_visible_item_with_a_min_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    row_scrolling_item_with_a_min_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    row_visible_item_with_a_min_content_width_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 30px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    row_scrolling_item_with_a_min_content_width_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    row_visible_item_with_a_min_content_width_flex_1_0_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 0 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    row_scrolling_item_with_a_min_content_width_flex_1_0_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 0 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    row_visible_item_with_a_min_content_width_flex_1_1_auto:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    row_scrolling_item_with_a_max_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:max-content;flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_with_a_max_content_width_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:max-content;flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_with_a_max_content_width_flex_1_0_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:max-content;flex:1 0 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_with_a_fit_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:fit-content;flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_with_a_fit_content_width_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:fit-content;flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_with_a_fit_content_width_flex_1_0_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:fit-content;flex:1 0 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_flex_0_1_auto:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="flex:0 1 auto;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_scrolling_item_flex_0_0_30px:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="flex:0 0 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 30.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 60.0, 36.0))];
    row_scrolling_item_flex_1_1_auto:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 auto;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_visible_item_with_a_max_content_width_flex_1_1_auto:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:max-content;flex:1 1 auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_visible_item_with_a_fit_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:fit-content;flex:1 1 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    row_visible_item_with_a_20px_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:20px;flex:1 1 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 20.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 50.0, 36.0))];
    row_visible_item_with_a_min_content_width_flex_0_1_auto:
        r#"<div id=o style="float:left;display:flex"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:0 1 auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    column_visible_item_with_a_30px_height_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex;flex-direction:column"><div id=b style="height:20px">b</div><div id=a style="height:30px;flex:1 1 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 128.0, 16.0)), ("b", (8.0, 8.0, 128.0, 20.0)), ("o", (8.0, 8.0, 128.0, 36.0))];
    column_visible_item_with_a_30px_height_flex_1_1_15px:
        r#"<div id=o style="float:left;display:flex;flex-direction:column"><div id=b style="height:20px">b</div><div id=a style="height:30px;flex:1 1 15px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 128.0, 16.0)), ("b", (8.0, 8.0, 128.0, 20.0)), ("o", (8.0, 8.0, 128.0, 36.0))];
    column_visible_item_with_a_30px_height_flex_1_0_0:
        r#"<div id=o style="float:left;display:flex;flex-direction:column"><div id=b style="height:20px">b</div><div id=a style="height:30px;flex:1 0 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 128.0, 16.0)), ("b", (8.0, 8.0, 128.0, 20.0)), ("o", (8.0, 8.0, 128.0, 36.0))];
    column_visible_item_with_a_30px_height_flex_1_0_15px:
        r#"<div id=o style="float:left;display:flex;flex-direction:column"><div id=b style="height:20px">b</div><div id=a style="height:30px;flex:1 0 15px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 128.0, 16.0)), ("b", (8.0, 8.0, 128.0, 20.0)), ("o", (8.0, 8.0, 128.0, 36.0))];
    column_visible_item_with_a_30px_height_flex_0_1_15px:
        r#"<div id=o style="float:left;display:flex;flex-direction:column"><div id=b style="height:20px">b</div><div id=a style="height:30px;flex:0 1 15px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 128.0, 16.0)), ("b", (8.0, 8.0, 128.0, 20.0)), ("o", (8.0, 8.0, 128.0, 36.0))];
    column_visible_item_with_a_4px_height_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex;flex-direction:column"><div id=b style="height:20px">b</div><div id=a style="height:4px;flex:1 1 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 128.0, 6.0)), ("b", (8.0, 8.0, 128.0, 20.0)), ("o", (8.0, 8.0, 128.0, 26.0))];
    column_visible_item_with_a_4px_height_flex_1_1_15px:
        r#"<div id=o style="float:left;display:flex;flex-direction:column"><div id=b style="height:20px">b</div><div id=a style="height:4px;flex:1 1 15px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 128.0, 15.0)), ("b", (8.0, 8.0, 128.0, 20.0)), ("o", (8.0, 8.0, 128.0, 35.0))];
    column_scrolling_item_with_a_30px_height_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex;flex-direction:column"><div id=b style="height:20px">b</div><div id=a style="height:30px;flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 128.0, 6.0)), ("b", (8.0, 8.0, 128.0, 20.0)), ("o", (8.0, 8.0, 128.0, 26.0))];
    column_visible_item_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex;flex-direction:column"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 128.0, 16.0)), ("b", (8.0, 8.0, 128.0, 20.0)), ("o", (8.0, 8.0, 128.0, 36.0))];
    min_content_container_scrolling_item_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex;width:min-content"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    min_content_container_scrolling_item_with_a_min_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex;width:min-content"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    min_content_container_scrolling_item_flex_1_1_0_with_a_max_width:
        r#"<div id=o style="float:left;display:flex;width:min-content"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 0;overflow:auto;max-width:50px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    max_content_container_scrolling_item_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex;width:max-content"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    max_content_container_scrolling_item_with_a_min_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex;width:max-content"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    max_content_container_scrolling_item_flex_1_1_0_with_a_min_width:
        r#"<div id=o style="float:left;display:flex;width:max-content"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 0;overflow:auto;min-width:100px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    fit_content_container_scrolling_item_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex;width:fit-content"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    fit_content_container_visible_item_with_a_min_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex;width:fit-content"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    fit_content_container_scrolling_item_flex_1_1_0_with_a_max_width:
        r#"<div id=o style="float:left;display:flex;width:fit-content"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 0;overflow:auto;max-width:50px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 50.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 80.0, 36.0))];
    inline_flex_container_in_narrow_room_scrolling_item_flex_1_1_0:
        r#"<div id=o style="display:inline-flex;max-width:60px"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 30.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 60.0, 36.0))];
    inline_flex_container_in_narrow_room_scrolling_item_with_a_max_content_width_flex_1_1_0:
        r#"<div id=o style="display:inline-flex;max-width:60px"><div id=b style="width:30px">b</div><div id=a style="width:max-content;flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 30.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 60.0, 36.0))];
    inline_flex_container_in_narrow_room_scrolling_item_flex_1_1_0_with_a_max_width:
        r#"<div id=o style="display:inline-flex;max-width:60px"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 0;overflow:auto;max-width:50px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 30.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 60.0, 36.0))];
    wrapping_max_content_container_scrolling_item_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex;flex-wrap:wrap;width:max-content"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 128.0, 16.0)), ("b", (8.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    wrapping_max_content_container_scrolling_item_with_a_min_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex;flex-wrap:wrap;width:max-content"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 0;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 48.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    wrapping_max_content_container_scrolling_item_flex_1_1_0_with_a_max_width:
        r#"<div id=o style="float:left;display:flex;flex-wrap:wrap;width:max-content"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 0;overflow:auto;max-width:50px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (38.0, 8.0, 50.0, 36.0)), ("b", (8.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 80.0, 36.0))];
    reversed_max_content_container_scrolling_item_flex_1_1_30px:
        r#"<div id=o style="float:left;display:flex;flex-direction:row-reverse"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 30px;overflow:auto;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 8.0, 128.0, 16.0)), ("b", (136.0, 8.0, 30.0, 16.0)), ("o", (8.0, 8.0, 158.0, 16.0))];
    reversed_max_content_container_visible_item_with_a_min_content_width_flex_1_1_0:
        r#"<div id=o style="float:left;display:flex;flex-direction:row-reverse"><div id=b style="width:30px">b</div><div id=a style="width:min-content;flex:1 1 0;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 8.0, 48.0, 36.0)), ("b", (56.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 78.0, 36.0))];
    reversed_max_content_container_scrolling_item_flex_1_1_0_with_a_max_width:
        r#"<div id=o style="float:left;display:flex;flex-direction:row-reverse"><div id=b style="width:30px">b</div><div id=a style="flex:1 1 0;overflow:auto;max-width:50px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 36.0)), ("b", (58.0, 8.0, 30.0, 36.0)), ("o", (8.0, 8.0, 80.0, 36.0))];
    column_item_with_a_max_width_keeps_a_stated_height_below_its_content_at_that_width:
        r#"<div id=o style="display:flex;flex-direction:column;width:200px"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;height:20px;max-width:40px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 40.0, 20.0)), ("b", (8.0, 8.0, 200.0, 20.0)), ("o", (8.0, 8.0, 200.0, 40.0))];
    column_item_with_a_percentage_max_width_keeps_a_stated_height_below_its_content_at_that_width:
        r#"<div id=o style="display:flex;flex-direction:column;width:200px"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;height:20px;max-width:20%;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 40.0, 20.0)), ("b", (8.0, 8.0, 200.0, 20.0)), ("o", (8.0, 8.0, 200.0, 40.0))];
    column_item_with_a_min_width_wider_than_the_container_is_as_tall_as_its_content_at_that_width:
        r#"<div id=o style="display:flex;flex-direction:column;width:60px"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;height:60px;min-width:100px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 100.0, 26.0)), ("b", (8.0, 8.0, 60.0, 20.0)), ("o", (8.0, 8.0, 60.0, 46.0))];
    column_item_with_a_min_width_wider_than_the_container_keeps_a_stated_height_below_its_content:
        r#"<div id=o style="display:flex;flex-direction:column;width:60px"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;height:20px;min-width:100px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 100.0, 20.0)), ("b", (8.0, 8.0, 60.0, 20.0)), ("o", (8.0, 8.0, 60.0, 40.0))];
    column_item_with_a_40px_width_keeps_a_stated_height_below_its_content_at_that_width:
        r#"<div id=o style="display:flex;flex-direction:column;width:200px"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;height:20px;width:40px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 40.0, 20.0)), ("b", (8.0, 8.0, 200.0, 20.0)), ("o", (8.0, 8.0, 200.0, 40.0))];
    column_item_with_a_width_above_its_max_width_keeps_a_stated_height_below_its_content_at_the_max_width:
        r#"<div id=o style="display:flex;flex-direction:column;width:200px"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;height:20px;width:80px;max-width:40px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 40.0, 20.0)), ("b", (8.0, 8.0, 200.0, 20.0)), ("o", (8.0, 8.0, 200.0, 40.0))];
    column_content_box_item_with_a_max_width_keeps_a_stated_height_below_its_content:
        r#"<div id=o style="display:flex;flex-direction:column;width:200px"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;height:20px;max-width:40px;box-sizing:content-box;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 48.0, 26.0)), ("b", (8.0, 8.0, 200.0, 20.0)), ("o", (8.0, 8.0, 200.0, 46.0))];
    column_item_with_a_max_width_and_an_80px_height_is_as_tall_as_its_content_at_that_width:
        r#"<div id=o style="display:flex;flex-direction:column;width:200px"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;height:80px;max-width:40px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 40.0, 36.0)), ("b", (8.0, 8.0, 200.0, 20.0)), ("o", (8.0, 8.0, 200.0, 56.0))];
    column_floated_container_item_with_a_max_width_keeps_a_stated_height_below_its_content:
        r#"<div id=o style="float:left;display:flex;flex-direction:column;width:200px"><div id=b style="height:20px">b</div><div id=a style="flex:1 1 0;height:20px;max-width:40px;padding:2px 3px;border:1px solid">ccc ddd eeee</div></div>"# =>
        [("a", (8.0, 28.0, 40.0, 20.0)), ("b", (8.0, 8.0, 200.0, 20.0)), ("o", (8.0, 8.0, 200.0, 40.0))];
}
