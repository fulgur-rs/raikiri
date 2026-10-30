//! A `width: stretch` item of a flex container that wraps its items onto
//! columns (CSS Sizing 4 §5.1, csswg-drafts#11784): while the lines are formed
//! the item is as wide as its container, and it then stretches to the line it
//! lands in, so a line made of stretching items alone is as wide as the
//! container and a line with a wider item is as wide as that item.
//!
//! The container itself is as wide as its widest item at `min-content`, not as
//! wide as the lines its height makes (the sum, which is its `max-content`
//! width).
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
    stretch_items_take_the_container_width_in_their_own_lines:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid;width:stretch">a</div><div id=b style="height:75px;border:3px solid;width:stretch">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (11.0, 11.0, 94.0, 75.0)), ("b", (105.0, 11.0, 94.0, 75.0))];
    stretch_items_leave_their_margins_out:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid;width:stretch;margin:0 7px 0 4px">a</div><div id=b style="height:75px;border:3px solid;width:stretch;margin:0 7px 0 4px">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (15.0, 11.0, 83.0, 75.0)), ("b", (109.0, 11.0, 83.0, 75.0))];
    stretch_items_stop_at_max_width:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid;width:stretch;max-width:60px">a</div><div id=b style="height:75px;border:3px solid;width:stretch;max-width:60px">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (11.0, 11.0, 60.0, 75.0)), ("b", (71.0, 11.0, 60.0, 75.0))];
    stretch_item_wider_than_the_container_by_min_width:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid;width:stretch;min-width:130px">a</div><div id=b style="height:75px;border:3px solid;width:stretch">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (11.0, 11.0, 130.0, 75.0)), ("b", (141.0, 11.0, 94.0, 75.0))];
    stretch_items_fill_the_content_box_of_a_padded_container:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem;padding:0 10px"><div id=a style="height:75px;border:3px solid;width:stretch">a</div><div id=b style="height:75px;border:3px solid;width:stretch">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (21.0, 11.0, 74.0, 75.0)), ("b", (95.0, 11.0, 74.0, 75.0))];
    stretch_item_text_wraps_at_the_container_width:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;align-content:start;font:20px/1 Ahem"><div id=a style="width:stretch;border:3px solid">aa bb cc dd</div><div id=b style="width:stretch;border:3px solid;height:60px">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (8.0, 8.0, 100.0, 86.0)), ("b", (108.0, 8.0, 100.0, 60.0))];
    webkit_fill_available_matches_stretch:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid;width:-webkit-fill-available">a</div><div id=b style="height:75px;border:3px solid;width:-webkit-fill-available">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (11.0, 11.0, 94.0, 75.0)), ("b", (105.0, 11.0, 94.0, 75.0))];
    single_line_container_is_unchanged:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem;flex-wrap:nowrap"><div id=a style="height:75px;border:3px solid;width:stretch;height:40px">a</div><div id=b style="height:75px;border:3px solid;width:stretch;height:40px">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (11.0, 11.0, 94.0, 40.0)), ("b", (11.0, 51.0, 94.0, 40.0))];
    wrap_reverse_container:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem;flex-wrap:wrap-reverse"><div id=a style="height:75px;border:3px solid;width:stretch">a</div><div id=b style="height:75px;border:3px solid;width:stretch">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (105.0, 11.0, 94.0, 75.0)), ("b", (11.0, 11.0, 94.0, 75.0))];
    align_content_stretch_container:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem;align-content:stretch"><div id=a style="height:75px;border:3px solid;width:stretch">a</div><div id=b style="height:75px;border:3px solid;width:stretch">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (11.0, 11.0, 94.0, 75.0)), ("b", (105.0, 11.0, 94.0, 75.0))];
    replaced_item_with_a_stretch_width:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><canvas id=a width=20 height=10 style="height:75px;width:stretch"></canvas><div id=b style="height:75px;border:3px solid;width:stretch">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (11.0, 11.0, 94.0, 75.0)), ("b", (105.0, 11.0, 94.0, 75.0))];
    auto_height_stretch_item:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid;width:stretch;height:auto">a</div><div id=b style="height:75px;border:3px solid;width:stretch">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (11.0, 11.0, 94.0, 26.0)), ("b", (105.0, 11.0, 94.0, 75.0))];
    line_with_a_wider_item_stretches_its_stretch_item_to_the_line:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:100px;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid;width:stretch">a</div><div id=o style="border:3px solid;width:150px"></div><div id=b style="height:75px;border:3px solid;width:stretch">b</div></div>"# =>
        [("c", (8.0, 8.0, 100.0, 100.0)), ("a", (11.0, 11.0, 150.0, 75.0)), ("o", (11.0, 86.0, 150.0, 6.0)), ("b", (161.0, 11.0, 94.0, 75.0))];
    shrink_to_fit_container_is_the_sum_of_its_lines:
        r#"<div id=c style="display:inline-flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid;width:stretch">a</div><div id=b style="height:75px;border:3px solid;width:stretch">bbb</div></div>"# =>
        [("c", (8.0, 8.0, 98.0, 100.0)), ("a", (11.0, 11.0, 92.0, 75.0)), ("b", (103.0, 11.0, 92.0, 75.0))];
    min_content_container_clamped_by_min_width:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:min-content;min-width:80px;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid;width:stretch">a</div><div id=b style="height:75px;border:3px solid;width:stretch">b</div></div>"# =>
        [("c", (8.0, 8.0, 80.0, 100.0)), ("a", (11.0, 11.0, 74.0, 75.0)), ("b", (85.0, 11.0, 74.0, 75.0))];
    min_content_container_is_as_wide_as_its_widest_item:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:min-content;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid;margin-left:10px">bbb</div></div>"# =>
        [("c", (8.0, 8.0, 82.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (47.0, 11.0, 66.0, 75.0))];
    min_content_container_counts_its_padding_and_border:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;width:min-content;height:100px;border:solid;padding:0 5px;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">b</div></div>"# =>
        [("c", (8.0, 8.0, 42.0, 100.0)), ("a", (16.0, 11.0, 26.0, 75.0)), ("b", (42.0, 11.0, 26.0, 75.0))];
    floated_container_shrinks_to_fit_its_lines:
        r#"<div style="float:left"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></div>"# =>
        [("c", (8.0, 8.0, 98.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    table_cell_keeps_the_explicit_width_of_a_wrapping_column_container:
        r#"<table style="border-spacing:0"><tr><td id=td style="padding:0"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:80px"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">b</div></div></td></tr></table>"# =>
        [("td", (8.0, 8.0, 80.0, 100.0)), ("c", (8.0, 8.0, 80.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 26.0, 75.0))];
    table_cell_fits_a_wrapping_column_container_with_auto_width:
        r#"<table style="border-spacing:0"><tr><td id=td style="padding:0"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></td></tr></table>"# =>
        [("td", (8.0, 8.0, 98.0, 100.0)), ("c", (8.0, 8.0, 98.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    float_keeps_the_explicit_width_of_a_wrapping_column_container:
        r#"<div id=f style="float:left"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:80px"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">b</div></div></div>"# =>
        [("f", (8.0, 8.0, 80.0, 100.0)), ("c", (8.0, 8.0, 80.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 26.0, 75.0))];
    inline_flex_wrapping_column_container_keeps_its_explicit_width:
        r#"<div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;display:inline-flex;width:80px"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">b</div></div>"# =>
        [("c", (8.0, 8.0, 80.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 26.0, 75.0))];
    table_cell_sizes_a_max_content_wrapping_column_container_by_the_sum_of_its_lines:
        r#"<table style="border-spacing:0"><tr><td id=td style="padding:0"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:max-content"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">b</div></div></td></tr></table>"# =>
        [("td", (8.0, 8.0, 58.0, 100.0)), ("c", (8.0, 8.0, 58.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 26.0, 75.0))];
    table_cell_fits_a_wrapping_column_container_within_its_max_width:
        r#"<table style="border-spacing:0"><tr><td id=td style="padding:0"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;max-width:60px"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">b</div></div></td></tr></table>"# =>
        [("td", (8.0, 8.0, 58.0, 100.0)), ("c", (8.0, 8.0, 58.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 26.0, 75.0))];
    absolutely_positioned_wrapping_column_container_is_as_wide_as_its_lines:
        r#"<div style="position:relative;width:400px"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;position:absolute"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></div>"# =>
        [("c", (8.0, 8.0, 98.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    table_max_width_keeps_the_explicit_width_of_a_wrapping_column_container:
        r#"<table style="max-width:60px;border-spacing:0"><tr><td id=td style="padding:0"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:90px"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></td></tr></table>"# =>
        [("td", (8.0, 8.0, 90.0, 100.0)), ("c", (8.0, 8.0, 90.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    fit_content_wrapping_column_container_takes_the_room_of_its_widest_item:
        r#"<div style="width:40px"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:fit-content"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></div>"# =>
        [("c", (8.0, 8.0, 72.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    fit_content_wrapping_column_container_in_a_wide_parent_is_the_sum_of_its_lines:
        r#"<div style="width:300px"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:fit-content"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></div>"# =>
        [("c", (8.0, 8.0, 98.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    percent_width_container_in_a_min_content_parent_counts_as_its_widest_item:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:50%"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></div>"# =>
        [("p", (8.0, 8.0, 72.0, 100.0)), ("c", (8.0, 8.0, 36.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    percent_width_container_in_a_float_counts_as_its_lines:
        r#"<div id=p style="float:left"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:50%"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></div>"# =>
        [("p", (8.0, 8.0, 98.0, 100.0)), ("c", (8.0, 8.0, 49.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    table_cell_fits_a_fit_content_wrapping_column_container:
        r#"<table style="max-width:60px;border-spacing:0"><tr><td id=td style="padding:0"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:fit-content"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></td></tr></table>"# =>
        [("td", (8.0, 8.0, 72.0, 100.0)), ("c", (8.0, 8.0, 72.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    calc_width_container_keeps_its_width_in_a_float:
        r#"<div id=p style="float:left"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid;align-content:start;font:20px/1 Ahem;width:calc(40px + 10px)"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">bbb</div></div></div>"# =>
        [("p", (8.0, 8.0, 50.0, 100.0)), ("c", (8.0, 8.0, 50.0, 100.0)), ("a", (11.0, 11.0, 26.0, 75.0)), ("b", (37.0, 11.0, 66.0, 75.0))];
    percent_height_items_with_ratios_keep_their_widths_in_a_min_content_parent:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;font:20px/1 Ahem"><div id=a style="height:50%;aspect-ratio:2"></div><div id=b style="height:50%;aspect-ratio:1"></div></div></div>"# =>
        [("p", (8.0, 8.0, 100.0, 100.0)), ("c", (8.0, 8.0, 100.0, 100.0)), ("a", (8.0, 8.0, 100.0, 50.0)), ("b", (8.0, 58.0, 100.0, 50.0))];
    canvas_item_with_a_percent_height_keeps_its_width_in_a_min_content_parent:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;font:20px/1 Ahem"><canvas id=a width=20 height=10 style="height:50%"></canvas></div></div>"# =>
        [("p", (8.0, 8.0, 100.0, 100.0)), ("c", (8.0, 8.0, 100.0, 100.0)), ("a", (8.0, 8.0, 100.0, 50.0))];
    canvas_item_with_a_percent_min_height_keeps_its_width_in_a_min_content_parent:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;font:20px/1 Ahem"><canvas id=a width=20 height=10 style="min-height:50%"></canvas></div></div>"# =>
        [("p", (8.0, 8.0, 100.0, 100.0)), ("c", (8.0, 8.0, 100.0, 100.0)), ("a", (8.0, 8.0, 100.0, 50.0))];
    canvas_item_with_a_percent_max_height_keeps_its_width_in_a_min_content_parent:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;font:20px/1 Ahem"><canvas id=a width=200 height=100 style="max-height:25%"></canvas></div></div>"# =>
        [("p", (8.0, 8.0, 50.0, 100.0)), ("c", (8.0, 8.0, 50.0, 100.0)), ("a", (8.0, 8.0, 50.0, 25.0))];
    flex_basis_percent_item_leaves_a_min_content_parent_at_its_natural_width:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;font:20px/1 Ahem"><canvas id=a width=20 height=10 style="flex-basis:50%"></canvas></div></div>"# =>
        [("p", (8.0, 8.0, 20.0, 100.0)), ("c", (8.0, 8.0, 20.0, 100.0)), ("a", (8.0, 8.0, 100.0, 50.0))];
    container_with_an_aspect_ratio_is_as_wide_as_its_ratio_says_at_min_content:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;font:20px/1 Ahem;aspect-ratio:1/2;height:100px"><div id=a style="height:40px;width:30px"></div><div id=b style="height:40px;width:20px"></div><div id=d style="height:40px;width:10px"></div></div></div>"# =>
        [("p", (8.0, 8.0, 50.0, 100.0)), ("c", (8.0, 8.0, 50.0, 100.0)), ("a", (8.0, 8.0, 30.0, 40.0)), ("b", (8.0, 48.0, 20.0, 40.0)), ("d", (43.0, 8.0, 10.0, 40.0))];
    stretch_height_item_with_a_ratio_widens_a_min_content_parent:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;font:20px/1 Ahem"><div id=a style="height:stretch;aspect-ratio:2"></div></div></div>"# =>
        [("p", (8.0, 8.0, 200.0, 100.0)), ("c", (8.0, 8.0, 200.0, 100.0)), ("a", (8.0, 8.0, 200.0, 100.0))];

    growing_ratio_item_adds_nothing_to_a_min_content_parent:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;font:20px/1 Ahem"><div id=a style="flex:1 1 auto;aspect-ratio:2"></div></div></div>"# =>
        [("p", (8.0, 8.0, 0.0, 100.0)), ("c", (8.0, 8.0, 0.0, 100.0)), ("a", (8.0, 8.0, 200.0, 100.0))];
    ratio_item_with_a_length_height_widens_a_min_content_parent:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;font:20px/1 Ahem"><div id=a style="height:40px;aspect-ratio:2"></div><div id=b style="height:40px;width:30px"></div></div></div>"# =>
        [("p", (8.0, 8.0, 80.0, 100.0)), ("c", (8.0, 8.0, 80.0, 100.0)), ("a", (8.0, 8.0, 80.0, 40.0)), ("b", (8.0, 48.0, 30.0, 40.0))];
    border_box_max_width_below_the_padding_and_border_keeps_them:
        r#"<div id=p style="width:min-content"><div id=c style="display:flex;flex-direction:column;flex-wrap:wrap;height:100px;box-sizing:border-box;max-width:0;padding:10px;border:3px solid;font:20px/1 Ahem"><div id=a style="height:75px;border:3px solid">a</div><div id=b style="height:75px;border:3px solid">b</div></div></div>"# =>
        [("p", (8.0, 8.0, 26.0, 100.0)), ("c", (8.0, 8.0, 26.0, 100.0)), ("a", (21.0, 21.0, 26.0, 74.0)), ("b", (47.0, 21.0, 26.0, 74.0))];
}
