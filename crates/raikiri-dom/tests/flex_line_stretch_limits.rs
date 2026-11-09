//! `stretch` as the cross-axis `min-*` or `max-*` size of a flex item resolves
//! against the cross size of the flex line the item sits on (CSS Sizing 4
//! §5.1, CSS Flexbox 1 §9.4). The line takes its size from the hypothetical
//! cross sizes of its items, where the keyword resolves against the container,
//! and the item is then sized again within its line. A line can be larger
//! than the container when an item on it is, and smaller when the container
//! wraps its items onto more lines.
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
    a_column_wrap_item_fills_the_line_a_wider_sibling_sets:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 55.0, 26.0)), ("d", (0.0, 26.0, 60.0, 10.0))];
    a_column_wrap_item_fills_the_line_of_an_earlier_wider_sibling:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=d style="width:60px;height:10px"></div><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("d", (0.0, 0.0, 60.0, 10.0)), ("c", (2.0, 10.0, 55.0, 26.0))];
    a_column_wrap_item_fills_the_container_beside_a_narrow_sibling:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:30px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 45.0, 26.0)), ("d", (0.0, 26.0, 30.0, 10.0))];
    a_column_wrap_item_alone_fills_the_container:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 45.0, 26.0))];
    a_column_wrap_item_on_its_own_line_fills_the_container:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=d style="width:60px;height:30px"></div><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("d", (0.0, 0.0, 60.0, 30.0)), ("c", (62.0, 0.0, 45.0, 26.0))];
    a_column_wrap_item_on_the_second_line_keeps_its_hypothetical_line:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:100px;height:40px"><div id=d style="width:20px;height:30px"></div><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 40.0)), ("d", (0.0, 0.0, 20.0, 30.0)), ("c", (22.0, 0.0, 95.0, 26.0))];
    a_column_wrap_item_on_the_second_line_after_a_gap:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:100px;height:40px;column-gap:10px"><div id=d style="width:20px;height:30px"></div><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 40.0)), ("d", (0.0, 0.0, 20.0, 30.0)), ("c", (32.0, 0.0, 95.0, 26.0))];
    a_column_wrap_item_fills_its_line_with_align_content_flex_start:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px;align-content:flex-start"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 55.0, 26.0)), ("d", (0.0, 26.0, 60.0, 10.0))];
    a_column_wrap_item_with_align_content_flex_start_beside_a_narrow_sibling:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px;align-content:flex-start"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:30px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 45.0, 26.0)), ("d", (0.0, 26.0, 30.0, 10.0))];
    a_column_wrap_item_with_align_content_center_beside_a_narrow_sibling:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px;align-content:center"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:30px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 45.0, 26.0)), ("d", (0.0, 26.0, 30.0, 10.0))];
    a_single_line_item_fills_a_definite_container_not_a_wider_sibling:
        r#"<div id=o style="display:flex;flex-flow:column nowrap;width:50px;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 45.0, 26.0)), ("d", (0.0, 26.0, 60.0, 10.0))];
    a_column_wrap_item_in_a_shrink_wrapped_container:
        r#"<div id=o style="display:inline-flex;flex-flow:column wrap;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 60.0, 40.0)), ("c", (2.0, 0.0, 55.0, 26.0)), ("d", (0.0, 26.0, 60.0, 10.0))];
    a_single_line_item_in_a_shrink_wrapped_container:
        r#"<div id=o style="display:inline-flex;flex-flow:column nowrap;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 60.0, 40.0)), ("c", (2.0, 0.0, 55.0, 26.0)), ("d", (0.0, 26.0, 60.0, 10.0))];
    a_max_stretch_follows_the_line:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="max-width:stretch;width:300px;height:20px;margin:0 3px 0 2px"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 55.0, 20.0)), ("d", (0.0, 20.0, 60.0, 10.0))];
    a_max_stretch_alone_follows_the_container:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="max-width:stretch;width:300px;height:20px;margin:0 3px 0 2px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 45.0, 20.0))];
    a_wrap_reverse_item_fills_its_line:
        r#"<div id=o style="display:flex;flex-flow:column wrap-reverse;width:50px;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (-8.0, 0.0, 55.0, 26.0)), ("d", (-10.0, 26.0, 60.0, 10.0))];
    a_column_reverse_wrap_item_fills_its_line:
        r#"<div id=o style="display:flex;flex-flow:column-reverse wrap;width:50px;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;border:3px solid;padding:0 2px;box-sizing:content-box"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 14.0, 55.0, 26.0)), ("d", (0.0, 4.0, 60.0, 10.0))];
    a_row_wrap_min_height_fills_a_definite_container:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;height:80px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 80.0)), ("c", (0.0, 2.0, 20.0, 75.0)), ("d", (20.0, 0.0, 20.0, 60.0))];
    a_row_wrap_min_height_fills_a_line_taller_than_the_container:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;height:80px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:100px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 80.0)), ("c", (0.0, 2.0, 20.0, 95.0)), ("d", (20.0, 0.0, 20.0, 100.0))];
    a_row_wrap_min_height_fills_the_line_in_an_auto_height_container:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 60.0)), ("c", (0.0, 2.0, 20.0, 55.0)), ("d", (20.0, 0.0, 20.0, 60.0))];
    a_single_line_min_height_fills_an_auto_height_container:
        r#"<div id=o style="display:flex;flex-flow:row nowrap;width:100px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 60.0)), ("c", (0.0, 2.0, 20.0, 55.0)), ("d", (20.0, 0.0, 20.0, 60.0))];
    a_row_wrap_min_height_fills_its_own_line:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px"><div id=d style="width:90px;height:60px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 90.0)), ("d", (0.0, 0.0, 90.0, 60.0)), ("c", (0.0, 62.0, 20.0, 25.0)), ("e", (20.0, 60.0, 20.0, 30.0))];
    a_row_wrap_min_height_on_the_second_line_of_a_definite_container:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;height:200px"><div id=d style="width:90px;height:60px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 90.0, 60.0)), ("c", (0.0, 62.0, 20.0, 195.0)), ("e", (20.0, 60.0, 20.0, 30.0))];
    a_row_wrap_min_height_on_the_second_line_after_a_gap:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;height:200px;row-gap:20px"><div id=d style="width:90px;height:60px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 90.0, 60.0)), ("c", (0.0, 82.0, 20.0, 195.0)), ("e", (20.0, 80.0, 20.0, 30.0))];
    a_row_wrap_max_height_follows_the_line:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px"><div id=c style="max-height:stretch;height:300px;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 305.0)), ("c", (0.0, 2.0, 20.0, 300.0)), ("d", (20.0, 0.0, 20.0, 60.0))];
    a_row_wrap_line_inside_padding_and_border:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;padding:5px;border:2px solid"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 74.0)), ("c", (7.0, 9.0, 20.0, 55.0)), ("d", (27.0, 7.0, 20.0, 60.0))];
    a_row_wrap_line_with_a_stretched_sibling:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div><div id=e style="width:20px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 60.0)), ("c", (0.0, 2.0, 20.0, 55.0)), ("d", (20.0, 0.0, 20.0, 60.0)), ("e", (40.0, 0.0, 20.0, 60.0))];
    a_definite_container_with_a_stretched_line:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;height:200px"><div id=d style="width:90px;min-height:40px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 90.0, 40.0)), ("c", (0.0, 42.0, 20.0, 195.0)), ("e", (20.0, 40.0, 20.0, 30.0))];
    a_definite_container_with_a_stretched_line_and_a_percentage_gap:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;height:200px;row-gap:10%"><div id=d style="width:90px;min-height:40px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 90.0, 40.0)), ("c", (0.0, 62.0, 20.0, 195.0)), ("e", (20.0, 60.0, 20.0, 30.0))];
    a_definite_container_with_two_unstretched_lines:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;height:200px"><div id=d style="width:90px;height:40px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 90.0, 40.0)), ("c", (0.0, 42.0, 20.0, 195.0)), ("e", (20.0, 40.0, 20.0, 30.0))];
    a_definite_container_with_align_content_space_between:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;height:200px;align-content:space-between"><div id=d style="width:90px;height:40px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 90.0, 40.0)), ("c", (0.0, 42.0, 20.0, 195.0)), ("e", (20.0, 40.0, 20.0, 30.0))];
    a_line_with_an_auto_margin_sibling:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;height:200px"><div id=d style="width:90px;height:40px;margin:auto 0"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 90.0, 40.0)), ("c", (0.0, 42.0, 20.0, 195.0)), ("e", (20.0, 40.0, 20.0, 30.0))];
    a_line_spaced_out_by_justify_content:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;justify-content:space-between"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div><div id=e style="width:90px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 90.0)), ("c", (0.0, 2.0, 20.0, 55.0)), ("d", (80.0, 0.0, 20.0, 60.0)), ("e", (0.0, 60.0, 90.0, 30.0))];
    a_row_reverse_wrap_line:
        r#"<div id=o style="display:flex;flex-flow:row-reverse wrap;width:100px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div><div id=e style="width:90px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 90.0)), ("c", (80.0, 2.0, 20.0, 55.0)), ("d", (60.0, 0.0, 20.0, 60.0)), ("e", (10.0, 60.0, 90.0, 30.0))];
    a_row_reverse_wrap_second_line:
        r#"<div id=o style="display:flex;flex-flow:row-reverse wrap;width:100px"><div id=e style="width:90px;height:30px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 90.0)), ("e", (10.0, 0.0, 90.0, 30.0)), ("c", (80.0, 32.0, 20.0, 55.0)), ("d", (60.0, 30.0, 20.0, 60.0))];
    two_stretch_items_share_a_line:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:60px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px"></div><div id=c2 style="min-width:stretch;width:0;height:20px;margin:0 1px"></div><div id=d style="width:70px;height:10px"></div><div id=e style="width:30px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 60.0)), ("c", (2.0, 0.0, 65.0, 20.0)), ("c2", (1.0, 20.0, 68.0, 20.0)), ("d", (0.0, 40.0, 70.0, 10.0)), ("e", (70.0, 0.0, 30.0, 30.0))];
    a_wrap_container_in_a_block:
        r#"<div style="width:300px"><div id=o style="display:flex;flex-flow:row wrap"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div></div>"# =>
        [("o", (0.0, 0.0, 300.0, 60.0)), ("c", (0.0, 2.0, 20.0, 55.0)), ("d", (20.0, 0.0, 20.0, 60.0))];
    min_and_max_stretch_follow_the_line:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="min-width:stretch;max-width:stretch;width:0;height:20px;margin:0 3px 0 2px"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 55.0, 20.0)), ("d", (0.0, 20.0, 60.0, 10.0))];
    an_item_with_text_follows_the_line:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="min-width:stretch;height:20px;margin:0 3px 0 2px">ab</div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 55.0, 20.0)), ("d", (0.0, 20.0, 60.0, 10.0))];
    an_absolutely_positioned_sibling_starts_no_line:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px;position:relative"><div id=a style="position:absolute;width:5px;height:5px"></div><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("a", (0.0, 0.0, 5.0, 5.0)), ("c", (2.0, 0.0, 55.0, 20.0)), ("d", (0.0, 20.0, 60.0, 10.0))];
    lines_packed_at_the_start_of_a_container_raised_by_its_minimum:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;min-height:200px;align-content:flex-start"><div id=d style="width:90px;height:40px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 90.0, 40.0)), ("c", (0.0, 42.0, 20.0, 25.0)), ("e", (20.0, 40.0, 20.0, 30.0))];
    a_single_line_fills_a_container_raised_by_its_minimum:
        r#"<div id=o style="display:flex;flex-flow:row nowrap;width:100px;min-height:200px"><div id=d style="width:20px;height:40px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 20.0, 40.0)), ("c", (20.0, 2.0, 20.0, 195.0))];
    a_single_wrapped_line_takes_the_room_a_container_minimum_leaves:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;min-height:200px"><div id=d style="width:20px;height:40px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 200.0)), ("d", (0.0, 0.0, 20.0, 40.0)), ("c", (20.0, 2.0, 20.0, 195.0))];
    a_column_wrap_item_in_a_container_raised_by_its_minimum:
        r#"<div id=o style="display:inline-flex;flex-flow:column wrap;height:40px;min-width:200px"><div id=d style="width:60px;height:30px"></div><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 40.0)), ("d", (0.0, 0.0, 60.0, 30.0)), ("c", (62.0, 0.0, 195.0, 20.0))];
    a_right_to_left_row_wrap_line:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;direction:rtl"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div><div id=e style="width:90px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 90.0)), ("c", (80.0, 2.0, 20.0, 55.0)), ("d", (60.0, 0.0, 20.0, 60.0)), ("e", (10.0, 60.0, 90.0, 30.0))];
    a_right_to_left_row_wrap_second_line:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;direction:rtl"><div id=e style="width:90px;height:30px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 90.0)), ("e", (10.0, 0.0, 90.0, 30.0)), ("c", (80.0, 32.0, 20.0, 55.0)), ("d", (60.0, 30.0, 20.0, 60.0))];
    a_right_to_left_row_reverse_wrap_second_line:
        r#"<div id=o style="display:flex;flex-flow:row-reverse wrap;width:100px;direction:rtl"><div id=e style="width:90px;height:30px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 90.0)), ("e", (0.0, 0.0, 90.0, 30.0)), ("c", (0.0, 32.0, 20.0, 55.0)), ("d", (20.0, 30.0, 20.0, 60.0))];
    a_right_to_left_column_wrap_line:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px;direction:rtl"><div id=d style="width:60px;height:30px"></div><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px"></div><div id=e style="width:20px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("d", (-10.0, 0.0, 60.0, 30.0)), ("c", (-58.0, 0.0, 45.0, 20.0)), ("e", (-30.0, 20.0, 20.0, 10.0))];
    a_line_with_a_sibling_sized_by_a_height_keyword:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:min-content"><div style="height:60px"></div></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 60.0)), ("c", (0.0, 2.0, 20.0, 55.0)), ("d", (20.0, 0.0, 20.0, 60.0))];
    a_relatively_offset_sibling_stays_in_its_line:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px"><div id=d style="width:20px;height:60px"></div><div id=e style="width:20px;height:10px;position:relative;left:-30px;top:10px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 60.0)), ("d", (0.0, 0.0, 20.0, 60.0)), ("e", (-10.0, 10.0, 20.0, 10.0)), ("c", (40.0, 2.0, 20.0, 55.0))];
    a_relatively_offset_item_fills_its_line:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px;position:relative;top:7px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 60.0)), ("c", (0.0, 9.0, 20.0, 55.0)), ("d", (20.0, 0.0, 20.0, 60.0))];
    a_line_with_a_stretched_sibling_held_by_its_maximum:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;max-height:30px"><div style="height:60px"></div></div><div id=e style="width:20px;height:50px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 50.0)), ("c", (0.0, 2.0, 20.0, 45.0)), ("d", (20.0, 0.0, 20.0, 30.0)), ("e", (40.0, 0.0, 20.0, 50.0))];
    a_container_with_a_zero_minimum:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;min-height:0"><div id=d style="width:90px"><div style="height:60px"></div></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 90.0)), ("d", (0.0, 0.0, 90.0, 60.0)), ("c", (0.0, 62.0, 20.0, 25.0)), ("e", (20.0, 60.0, 20.0, 30.0))];
    percentage_padding_resolves_against_the_container:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 3px 0 2px;padding:0 10%;box-sizing:content-box"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (2.0, 0.0, 55.0, 20.0)), ("d", (0.0, 20.0, 60.0, 10.0))];
    percentage_margins_resolve_against_the_container:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;margin:0 10%"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (5.0, 0.0, 50.0, 20.0)), ("d", (0.0, 20.0, 60.0, 10.0))];
    a_percentage_fit_content_maximum_resolves_against_the_container:
        r#"<div id=o style="display:flex;flex-flow:column wrap;width:50px;height:40px"><div id=c style="min-width:stretch;width:0;height:20px;max-width:fit-content(50%)"></div><div id=d style="width:60px;height:10px"></div></div>"# =>
        [("o", (0.0, 0.0, 50.0, 40.0)), ("c", (0.0, 0.0, 60.0, 20.0)), ("d", (0.0, 20.0, 60.0, 10.0))];
    nested_wrap_containers_fill_their_lines:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:200px"><div id=p style="display:flex;flex-flow:row wrap;width:100px;min-height:stretch;margin:1px 0"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div><div id=e style="width:20px;height:90px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 90.0)), ("p", (0.0, 1.0, 100.0, 88.0)), ("c", (0.0, 3.0, 20.0, 83.0)), ("d", (20.0, 1.0, 20.0, 60.0)), ("e", (100.0, 0.0, 20.0, 90.0))];
    reversed_lines_that_all_hold_a_minimum_share_the_room:
        r#"<div id=o style="display:flex;flex-flow:row wrap-reverse;width:100px;min-height:100px"><div id=c style="min-height:stretch;height:20px;width:90px;margin:2px 0 3px"></div><div id=c2 style="min-height:stretch;height:0;width:20px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 100.0)), ("c", (0.0, 39.5, 90.0, 57.5)), ("c2", (0.0, 0.0, 20.0, 37.5))];
    reversed_columns_that_all_hold_a_minimum_share_the_room:
        r#"<div id=o style="display:inline-flex;flex-flow:column wrap-reverse;height:50px;min-width:200px"><div id=c style="min-width:stretch;width:20px;height:40px;margin:0 3px 0 2px"></div><div id=c2 style="min-width:stretch;width:0;height:20px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 50.0)), ("c", (2.0, 0.0, 195.0, 40.0)), ("c2", (-200.0, 0.0, 200.0, 20.0))];
    right_to_left_columns_that_all_hold_a_minimum_share_the_room:
        r#"<div id=o style="display:inline-flex;flex-flow:column wrap;height:50px;min-width:200px;direction:rtl"><div id=c style="min-width:stretch;width:20px;height:40px;margin:0 3px 0 2px"></div><div id=c2 style="min-width:stretch;width:0;height:20px"></div></div>"# =>
        [("o", (0.0, 0.0, 200.0, 50.0)), ("c", (2.0, 0.0, 195.0, 40.0)), ("c2", (-200.0, 0.0, 200.0, 20.0))];
    lines_that_all_hold_a_minimum_share_the_room:
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;min-height:100px"><div id=c style="min-height:stretch;height:20px;width:90px;margin:2px 0 3px"></div><div id=c2 style="min-height:stretch;height:0;width:20px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 100.0)), ("c", (0.0, 2.0, 90.0, 57.5)), ("c2", (0.0, 62.5, 20.0, 37.5))];
    a_single_line_in_a_scroll_container:
        r#"<div id=o style="display:flex;overflow-x:scroll;width:100px"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"# =>
        [("o", (0.0, 0.0, 100.0, 60.0)), ("c", (0.0, 2.0, 20.0, 55.0)), ("d", (20.0, 0.0, 20.0, 60.0))];
}

/// A container whose `min-height` makes it taller than its lines shares the
/// room between them. An item on a line that only takes a share keeps the
/// limits resolved against the container: the flex algorithm grows lines only
/// from the sizes of their items, so the minimum of the item would take the
/// room the other line shares. Chromium makes the item 90px tall here.
#[test]
fn a_line_sharing_the_room_of_a_container_minimum_keeps_the_container_limits() {
    check(
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;min-height:200px"><div id=d style="width:90px;height:40px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"#,
        &[
            ("o", (0.0, 0.0, 100.0, 200.0)),
            ("d", (0.0, 0.0, 90.0, 40.0)),
            ("c", (0.0, 107.0, 20.0, 0.0)),
            ("e", (20.0, 105.0, 20.0, 30.0)),
        ],
    );
}

/// As above, with the other line filled by an item stretched to it and held
/// by a `stretch` minimum. The stretched item is its line already and is not
/// placed again, so its line has no minimum that keeps its share. Chromium
/// makes `#c2` 50px tall here.
#[test]
fn a_line_filled_by_a_stretched_item_with_a_minimum_keeps_the_container_limits() {
    check(
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;min-height:100px"><div id=c style="min-height:stretch;width:90px"></div><div id=c2 style="min-height:stretch;height:0;width:20px"></div></div>"#,
        &[
            ("o", (0.0, 0.0, 100.0, 100.0)),
            ("c", (0.0, 0.0, 90.0, 50.0)),
            ("c2", (0.0, 50.0, 20.0, 0.0)),
        ],
    );
}

/// As the first case, with the other line filled by an item stretched to it.
#[test]
fn a_stretched_line_sharing_the_room_of_a_container_minimum_keeps_the_container_limits() {
    check(
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;min-height:200px"><div id=d style="width:90px;min-height:40px"></div><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=e style="width:20px;height:30px"></div></div>"#,
        &[
            ("o", (0.0, 0.0, 100.0, 200.0)),
            ("d", (0.0, 0.0, 90.0, 105.0)),
            ("c", (0.0, 107.0, 20.0, 0.0)),
            ("e", (20.0, 105.0, 20.0, 30.0)),
        ],
    );
}

/// An item aligned by its baseline keeps the limits resolved against the
/// container: the lines are found by laying the container out again with
/// the item at the start and at the end of its line, which leaves out the
/// baseline the item is placed by. Chromium makes the item 58px tall here.
#[test]
fn a_baseline_aligned_item_keeps_the_container_limits() {
    check(
        r#"<div id=o style="display:flex;flex-flow:row wrap;width:100px;align-items:baseline"><div id=c style="min-height:stretch;height:0;width:20px;margin:2px 0 3px"></div><div id=d style="width:20px;height:60px"></div></div>"#,
        &[
            ("c", (0.0, 60.0, 20.0, 0.0)),
            ("d", (20.0, 0.0, 20.0, 60.0)),
        ],
    );
}
