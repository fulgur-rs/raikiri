//! The `max-width` of a table counts in the intrinsic size of its parent and in
//! the width of the table itself. A table that shrinks to fit, or that a
//! `max-content` or `fit-content` width sizes, is as wide as its content up to
//! its `max-width`, and no narrower than its min-content width; its text wraps
//! at that width. A parent measured at its min-content or max-content width,
//! an inline block, a float, a flex or grid container, a box taken out of the
//! flow and a table cell add that width and not the width of the content.
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
    an_inline_block_takes_the_max_width_of_a_max_content_table_and_the_text_wraps:
        r#"<span id=o style="display:inline-block"><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></span>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_min_content_block_counts_a_max_content_table_up_to_its_max_width:
        r#"<div id=o style="width:min-content"><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_max_content_block_counts_a_max_content_table_up_to_its_max_width:
        r#"<div id=o style="width:max-content"><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    an_inline_block_counts_an_auto_table_up_to_its_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_float_counts_a_max_content_table_up_to_its_max_width:
        r#"<div id=o style="float:left"><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_max_content_table_in_a_block_is_held_to_its_max_width:
        r#"<table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_flex_container_leaves_a_max_content_table_item_at_its_max_width:
        r#"<div id=o style="display:flex"><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 784.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    an_inline_flex_container_counts_a_max_content_table_item_up_to_its_max_width:
        r#"<div id=o style="display:inline-flex"><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_max_content_grid_counts_a_max_content_table_up_to_its_max_width:
        r#"<div id=o style="display:grid;width:max-content"><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    an_absolute_box_counts_a_max_content_table_up_to_its_max_width:
        r#"<div id=o style="position:absolute"><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_width_over_the_max_width_is_held_to_it_in_an_inline_block:
        r#"<div id=o style="display:inline-block"><table id=t style="width:100px;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_min_width_over_the_max_width_decides_a_max_content_table:
        r#"<div id=o style="display:inline-block"><table id=t style="width:max-content;max-width:70px;min-width:90px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 90.0, 20.0)), ("o", (8.0, 8.0, 90.0, 20.0)), ("t", (8.0, 8.0, 90.0, 20.0))];
    the_border_counts_in_the_max_width_of_a_max_content_table:
        r#"<div id=o style="display:inline-block"><table id=t style="width:max-content;max-width:70px;border:5px solid"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (13.0, 13.0, 60.0, 30.0)), ("o", (8.0, 8.0, 70.0, 40.0)), ("t", (8.0, 8.0, 70.0, 40.0))];
    a_max_width_below_the_columns_keeps_the_min_content_width:
        r#"<div id=o style="display:inline-block"><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB</td><td id=e>CCCC DDDD</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 40.0, 20.0)), ("e", (48.0, 8.0, 40.0, 20.0)), ("o", (8.0, 8.0, 80.0, 20.0)), ("t", (8.0, 8.0, 80.0, 20.0))];
    a_fit_content_table_is_held_to_its_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="width:fit-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_max_content_table_in_a_cell_is_held_to_its_max_width:
        r#"<table id=u><tr><td id=c><table id=t style="width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></td></tr></table>"# =>
        [("c", (8.0, 8.0, 70.0, 30.0)), ("d", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0)), ("u", (8.0, 8.0, 70.0, 30.0))];
    a_caption_wraps_at_the_max_width_of_a_max_content_table:
        r#"<div id=o style="display:inline-block"><table id=t style="width:max-content;max-width:70px"><caption id=k>AAAA BBBB CCCC</caption><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 38.0, 70.0, 30.0)), ("k", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 60.0))];
    a_max_width_below_the_longest_word_keeps_the_min_content_width:
        r#"<div id=o style="display:inline-block"><table id=t style="width:max-content;max-width:30px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 40.0, 30.0)), ("o", (8.0, 8.0, 40.0, 30.0)), ("t", (8.0, 8.0, 40.0, 30.0))];
    a_percentage_max_width_resolves_against_the_shrink_wrapped_parent:
        r#"<div id=o style="display:inline-block;max-width:200px"><table id=t style="width:max-content;max-width:70%"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 98.0, 20.0)), ("o", (8.0, 8.0, 140.0, 20.0)), ("t", (8.0, 8.0, 98.0, 20.0))];
    margins_add_to_what_a_max_content_table_counts_up_to_its_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="width:max-content;max-width:70px;margin:0 10px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (18.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 90.0, 30.0)), ("t", (18.0, 8.0, 70.0, 30.0))];
    an_inline_table_counts_up_to_its_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:70px;display:inline-table"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 32.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_calc_max_width_limits_a_max_content_table:
        r#"<div id=o style="display:inline-block"><table id=t style="width:max-content;max-width:calc(30px + 40px)"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_max_width_over_the_max_content_width_leaves_the_table_as_wide_as_its_content:
        r#"<div id=o style="display:inline-block;width:min-content"><table id=t style="width:max-content;max-width:100px"><tr><td id=d>AAAA BBBB</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 90.0, 10.0)), ("o", (8.0, 8.0, 90.0, 10.0)), ("t", (8.0, 8.0, 90.0, 10.0))];
    a_fixed_layout_max_content_table_is_held_to_its_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="table-layout:fixed;width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_fixed_layout_auto_table_is_held_to_its_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="table-layout:fixed;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_fixed_layout_table_is_held_to_its_max_width_below_its_width:
        r#"<div id=o style="display:inline-block"><table id=t style="table-layout:fixed;width:200px;max-width:70px"><tr><td id=d style="width:100px">AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 100.0, 20.0)), ("o", (8.0, 8.0, 100.0, 20.0)), ("t", (8.0, 8.0, 100.0, 20.0))];
    a_max_width_over_the_max_content_width_does_not_change_a_max_content_table:
        r#"<div id=o style="display:inline-block"><table id=t style="width:max-content;max-width:200px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 140.0, 10.0)), ("o", (8.0, 8.0, 140.0, 10.0)), ("t", (8.0, 8.0, 140.0, 10.0))];
    a_max_width_between_the_sizes_limits_an_auto_table:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:100px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 100.0, 20.0)), ("o", (8.0, 8.0, 100.0, 20.0)), ("t", (8.0, 8.0, 100.0, 20.0))];
    a_max_width_equal_to_the_max_content_width_does_not_wrap_the_table:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:140px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 140.0, 10.0)), ("o", (8.0, 8.0, 140.0, 10.0)), ("t", (8.0, 8.0, 140.0, 10.0))];
    a_max_width_just_under_the_max_content_width_wraps_the_table:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:139px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 139.0, 20.0)), ("o", (8.0, 8.0, 139.0, 20.0)), ("t", (8.0, 8.0, 139.0, 20.0))];
    a_max_width_limits_a_table_of_two_columns:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:100px"><tr><td id=d>AAAA BBBB</td><td id=e>CCCC DDDD</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 50.0, 20.0)), ("e", (58.0, 8.0, 50.0, 20.0)), ("o", (8.0, 8.0, 100.0, 20.0)), ("t", (8.0, 8.0, 100.0, 20.0))];
    a_max_width_below_two_columns_keeps_their_min_content_widths:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:60px"><tr><td id=d>AAAA BBBB</td><td id=e>CCCC DDDD</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 40.0, 20.0)), ("e", (48.0, 8.0, 40.0, 20.0)), ("o", (8.0, 8.0, 80.0, 20.0)), ("t", (8.0, 8.0, 80.0, 20.0))];
    a_max_width_limits_a_table_with_a_cell_that_states_its_width:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:100px"><tr><td id=d style="width:60px">AAAA BBBB</td><td id=e>CCCC DDDD</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 60.0, 20.0)), ("e", (68.0, 8.0, 40.0, 20.0)), ("o", (8.0, 8.0, 100.0, 20.0)), ("t", (8.0, 8.0, 100.0, 20.0))];
    a_max_width_limits_a_table_with_a_percentage_column:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:100px"><tr><td id=d style="width:50%">AAAA BBBB</td><td id=e>CCCC DDDD</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 50.0, 20.0)), ("e", (58.0, 8.0, 50.0, 20.0)), ("o", (8.0, 8.0, 100.0, 20.0)), ("t", (8.0, 8.0, 100.0, 20.0))];
    padding_stays_inside_the_max_width_of_a_table:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:70px;padding:5px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (13.0, 13.0, 60.0, 30.0)), ("o", (8.0, 8.0, 70.0, 40.0)), ("t", (8.0, 8.0, 70.0, 40.0))];
    a_border_stays_inside_the_max_width_of_a_table:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:70px;border:4px solid"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (12.0, 12.0, 62.0, 30.0)), ("o", (8.0, 8.0, 70.0, 38.0)), ("t", (8.0, 8.0, 70.0, 38.0))];
    border_spacing_stays_inside_the_max_width_of_a_table:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:70px;border-spacing:6px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (14.0, 14.0, 58.0, 30.0)), ("o", (8.0, 8.0, 70.0, 42.0)), ("t", (8.0, 8.0, 70.0, 42.0))];
    a_content_box_table_keeps_its_padding_inside_the_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:70px;box-sizing:content-box;padding:5px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (13.0, 13.0, 70.0, 30.0)), ("o", (8.0, 8.0, 80.0, 40.0)), ("t", (8.0, 8.0, 80.0, 40.0))];
    a_border_box_table_keeps_its_padding_inside_the_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:70px;box-sizing:border-box;padding:5px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (13.0, 13.0, 60.0, 30.0)), ("o", (8.0, 8.0, 70.0, 40.0)), ("t", (8.0, 8.0, 70.0, 40.0))];
    a_flex_container_holds_a_table_item_to_its_max_width:
        r#"<div id=o style="display:flex;width:200px"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 200.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    an_inline_flex_container_counts_a_table_item_up_to_its_max_width:
        r#"<div id=o style="display:inline-flex"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_grid_container_holds_a_table_item_to_its_max_width:
        r#"<div id=o style="display:grid;grid-template-columns:auto 1fr;width:300px"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table><div id=z>x</div></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 300.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0)), ("z", (78.0, 8.0, 230.0, 30.0))];
    an_inline_grid_container_counts_a_table_item_up_to_its_max_width:
        r#"<div id=o style="display:inline-grid"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_float_counts_a_table_up_to_its_max_width:
        r#"<div id=o style="float:left"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    an_absolute_box_counts_a_table_up_to_its_max_width:
        r#"<div id=o style="position:absolute"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_table_in_a_cell_is_held_to_its_max_width:
        r#"<table id=u><tr><td id=c><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></td></tr></table>"# =>
        [("c", (8.0, 8.0, 70.0, 30.0)), ("d", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0)), ("u", (8.0, 8.0, 70.0, 30.0))];
    a_max_content_table_counts_a_table_cell_child_up_to_its_max_width:
        r#"<table id=u style="width:max-content"><tr><td id=c><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></td></tr></table>"# =>
        [("c", (8.0, 8.0, 70.0, 30.0)), ("d", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0)), ("u", (8.0, 8.0, 70.0, 30.0))];
    a_fit_content_block_counts_a_table_up_to_its_max_width:
        r#"<div id=o style="width:fit-content"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_min_content_block_adds_the_longest_word_of_a_table_under_its_max_width:
        r#"<div id=o style="width:min-content"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 40.0, 30.0)), ("o", (8.0, 8.0, 40.0, 30.0)), ("t", (8.0, 8.0, 40.0, 30.0))];
    a_max_content_block_counts_a_table_up_to_its_max_width:
        r#"<div id=o style="width:max-content"><table id=t style="max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_min_content_block_counts_a_table_up_to_its_min_content_width:
        r#"<div id=o style="width:min-content"><table id=t style="max-width:100px;min-width:20px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 40.0, 30.0)), ("o", (8.0, 8.0, 40.0, 30.0)), ("t", (8.0, 8.0, 40.0, 30.0))];
    a_min_width_over_the_max_width_decides_an_auto_table:
        r#"<div id=o style="display:inline-block"><table id=t style="min-width:100px;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 100.0, 20.0)), ("o", (8.0, 8.0, 100.0, 20.0)), ("t", (8.0, 8.0, 100.0, 20.0))];
    a_min_width_under_the_max_width_leaves_the_max_width_to_an_auto_table:
        r#"<div id=o style="display:inline-block"><table id=t style="min-width:50px;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_min_width_under_the_max_width_leaves_the_max_width_to_a_max_content_table:
        r#"<div id=o style="display:inline-block"><table id=t style="width:max-content;min-width:50px;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_min_content_table_under_a_larger_max_width_stays_at_its_longest_word:
        r#"<div id=o style="display:inline-block"><table id=t style="width:min-content;max-width:100px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 40.0, 30.0)), ("o", (8.0, 8.0, 40.0, 30.0)), ("t", (8.0, 8.0, 40.0, 30.0))];
    a_max_width_under_the_longest_word_leaves_a_min_content_table_at_it:
        r#"<div id=o style="display:inline-block"><table id=t style="width:min-content;max-width:20px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 40.0, 30.0)), ("o", (8.0, 8.0, 40.0, 30.0)), ("t", (8.0, 8.0, 40.0, 30.0))];
    an_inline_table_in_an_inline_block_counts_up_to_its_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="display:inline-table;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 32.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_max_content_inline_table_in_an_inline_block_counts_up_to_its_max_width:
        r#"<div id=o style="display:inline-block"><table id=t style="display:inline-table;width:max-content;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 70.0, 32.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    an_inline_table_in_a_line_is_held_to_its_max_width:
        r#"<table id=t style="display:inline-table;max-width:70px"><tr><td id=d>AAAA BBBB CCCC</td></tr></table>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_percentage_max_width_limits_a_table_in_a_sized_inline_block:
        r#"<div id=o style="display:inline-block;width:200px"><table id=t style="max-width:50%"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 100.0, 20.0)), ("o", (8.0, 8.0, 200.0, 20.0)), ("t", (8.0, 8.0, 100.0, 20.0))];
    a_percentage_max_width_limits_a_table_in_a_shrink_wrapped_inline_block:
        r#"<div id=o style="display:inline-block"><table id=t style="max-width:50%"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 70.0, 30.0)), ("o", (8.0, 8.0, 140.0, 30.0)), ("t", (8.0, 8.0, 70.0, 30.0))];
    a_percentage_max_width_is_resolved_in_a_min_content_block:
        r#"<div id=o style="width:min-content"><table id=t style="max-width:50%"><tr><td id=d>AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("d", (8.0, 8.0, 40.0, 30.0)), ("o", (8.0, 8.0, 40.0, 30.0)), ("t", (8.0, 8.0, 40.0, 30.0))];
}
