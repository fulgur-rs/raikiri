//! A box whose `width` is the keyword `min-content` or `max-content` adds the
//! width the keyword names to the intrinsic size of its parent, whichever of the
//! two sizes the parent is measured at: a `max-content` child that a `max-width`
//! limits adds that limit to a parent at its min-content width, not the width of
//! its longest word, and a `min-content` child adds its longest word to a parent
//! at its max-content width. The margins, the padding and the borders of the
//! child count in what it adds. A table is such a child as well; the cells of
//! a table and the boxes of a vertical writing mode keep the sizes they had.
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
    a_min_content_row_flex_container_counts_a_max_content_item_up_to_its_max_width:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_block_counts_a_max_content_child_up_to_its_max_width:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_grid_counts_a_max_content_item_up_to_its_max_width:
        r#"<div id=a style="display:grid;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_fit_content_item_adds_its_longest_word_to_a_min_content_row_container:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:fit-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0)), ("b", (8.0, 8.0, 40.0, 20.0))];
    a_min_content_item_adds_its_longest_word_to_a_min_content_row_container:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:min-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0)), ("b", (8.0, 8.0, 40.0, 20.0))];
    a_max_width_below_the_longest_word_limits_a_max_content_item:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:max-content;max-width:30px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 20.0)), ("b", (8.0, 8.0, 30.0, 20.0))];
    a_min_width_raises_a_max_content_item_of_a_min_content_container:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:max-content;min-width:100px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0)), ("b", (8.0, 8.0, 100.0, 10.0))];
    a_min_content_inline_block_counts_a_max_content_child_up_to_its_max_width:
        r#"<div id=a style="display:inline-block;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_column_flex_container_counts_a_max_content_item_up_to_its_max_width:
        r#"<div id=a style="display:flex;flex-direction:column;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_block_counts_a_max_content_grandchild_up_to_its_max_width:
        r#"<div id=a style="display:block;width:min-content"><div id=w><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("w", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_max_content_flex_basis_item_adds_its_longest_word_to_a_min_content_container:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="flex-basis:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0)), ("b", (8.0, 8.0, 40.0, 20.0))];
    a_max_content_row_flex_container_counts_a_max_content_item_up_to_its_max_width:
        r#"<div id=a style="display:flex;width:max-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_max_content_block_counts_a_max_content_child_up_to_its_max_width:
        r#"<div id=a style="display:block;width:max-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_stretch_child_adds_its_longest_word_to_a_min_content_block:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:stretch;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0)), ("b", (8.0, 8.0, 40.0, 20.0))];
    margins_add_to_the_max_content_contribution_of_a_block_child:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50px;margin:0 5px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 20.0)), ("b", (13.0, 8.0, 50.0, 20.0))];
    border_box_padding_stays_inside_the_max_width_of_a_max_content_child:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50px;padding:0 5px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    content_box_padding_adds_to_the_max_width_of_a_max_content_child:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50px;padding:0 5px;box-sizing:content-box">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 20.0)), ("b", (8.0, 8.0, 60.0, 20.0))];
    a_fit_content_child_adds_its_longest_word_to_a_min_content_block:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:fit-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0)), ("b", (8.0, 8.0, 40.0, 20.0))];
    a_max_width_above_the_longest_word_limits_a_longer_max_content_child:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:60px">AAAA BBBB CCCC</div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 30.0)), ("b", (8.0, 8.0, 60.0, 30.0))];
    a_max_width_below_the_longest_word_limits_a_max_content_block_child:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:20px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 20.0, 20.0)), ("b", (8.0, 8.0, 20.0, 20.0))];
    a_non_shrinking_max_content_item_counts_up_to_its_max_width:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:max-content;max-width:50px;flex-shrink:0">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_wrapping_flex_container_counts_a_max_content_item_up_to_its_max_width:
        r#"<div id=a style="display:flex;width:min-content;flex-wrap:wrap"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_inline_flex_container_counts_a_max_content_item_up_to_its_max_width:
        r#"<div id=a style="display:inline-flex;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_container_counts_a_max_content_item_that_holds_an_inline_block:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:max-content;max-width:50px"><div style="width:10px;height:10px;display:inline-block"></div></div></div>"# =>
        [("a", (8.0, 8.0, 10.0, 12.0)), ("b", (8.0, 8.0, 10.0, 12.0))];
    a_min_content_child_adds_its_longest_word_to_a_max_content_block:
        r#"<div id=a style="display:block;width:max-content"><div id=b style="width:min-content">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0)), ("b", (8.0, 8.0, 40.0, 20.0))];
    a_min_content_item_adds_its_longest_word_to_a_max_content_row_container:
        r#"<div id=a style="display:flex;width:max-content"><div id=b style="width:min-content">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0)), ("b", (8.0, 8.0, 40.0, 20.0))];
    a_min_content_child_adds_its_longest_word_to_a_max_content_inline_block:
        r#"<div id=a style="display:inline-block;width:max-content"><div id=b style="width:min-content">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0)), ("b", (8.0, 8.0, 40.0, 20.0))];
    a_min_width_raises_a_max_content_child_of_a_min_content_block:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;min-width:100px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 10.0)), ("b", (8.0, 8.0, 100.0, 10.0))];
    a_wider_sibling_decides_the_width_of_a_min_content_block:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div><div id=c>CCCCCC</div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 30.0)), ("b", (8.0, 8.0, 50.0, 20.0)), ("c", (8.0, 28.0, 60.0, 10.0))];
    a_min_content_block_counts_a_max_content_float_up_to_its_max_width:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="float:left;width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 0.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_row_container_sums_its_max_content_items_up_to_their_max_widths:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div><div id=c style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0)), ("c", (58.0, 8.0, 50.0, 20.0))];
    margins_add_to_the_max_content_contribution_of_a_row_flex_item:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:max-content;max-width:50px;margin:0 5px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 20.0)), ("b", (13.0, 8.0, 50.0, 20.0))];
    margins_add_to_the_max_content_contribution_of_a_wrapping_flex_item:
        r#"<div id=a style="display:flex;flex-wrap:wrap;width:min-content"><div id=b style="width:max-content;max-width:50px;margin:0 5px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 20.0)), ("b", (13.0, 8.0, 50.0, 20.0))];
    a_min_content_block_counts_a_max_content_block_child_with_block_content:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50px"><div id=c>AAAA BBBB</div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0)), ("c", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_block_counts_a_max_content_inline_block_up_to_its_max_width:
        r#"<div id=a style="display:block;width:min-content"><span id=b style="display:inline-block;width:max-content;max-width:50px">AAAA BBBB</span></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_table_counts_a_max_content_cell_child_up_to_its_max_width:
        r#"<table id=a style="border-spacing:0;width:min-content"><tr><td id=c style="padding:0"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></td></tr></table>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("c", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_block_counts_a_max_content_flex_container_up_to_its_max_width:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="display:flex;width:max-content;max-width:50px"><div id=c>AAAA BBBB</div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0)), ("c", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_block_counts_a_max_content_grid_container_up_to_its_max_width:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="display:grid;width:max-content;max-width:50px"><div id=c>AAAA BBBB</div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0)), ("c", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_block_counts_the_whole_line_of_a_max_content_child:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 90.0, 10.0)), ("b", (8.0, 8.0, 90.0, 10.0))];
    a_min_content_row_container_counts_the_whole_line_of_a_max_content_item:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:max-content">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 90.0, 10.0)), ("b", (8.0, 8.0, 90.0, 10.0))];
    a_min_width_raises_a_min_content_child_of_a_max_content_block:
        r#"<div id=a style="display:block;width:max-content"><div id=b style="width:min-content;min-width:60px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 20.0)), ("b", (8.0, 8.0, 60.0, 20.0))];
    a_fit_content_block_counts_a_max_content_child_up_to_its_max_width:
        r#"<div id=a style="display:block;width:fit-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    padding_and_border_add_to_the_max_width_of_a_content_box_max_content_child:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50px;padding:0 5px;border:2px solid;box-sizing:content-box">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 64.0, 24.0)), ("b", (8.0, 8.0, 64.0, 24.0))];
    a_wrapper_with_padding_adds_it_to_a_max_content_child_up_to_its_max_width:
        r#"<div id=a style="display:block;width:min-content"><div id=w style="padding:0 7px"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div></div>"# =>
        [("a", (8.0, 8.0, 64.0, 20.0)), ("w", (8.0, 8.0, 64.0, 20.0)), ("b", (15.0, 8.0, 50.0, 20.0))];
    a_margin_adds_to_the_max_content_contribution_of_a_block_child:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50px;margin-left:20px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 70.0, 20.0)), ("b", (28.0, 8.0, 50.0, 20.0))];
    a_min_content_item_that_does_not_shrink_stays_at_its_longest_word:
        r#"<div id=a style="display:flex;width:min-content"><div id=b style="width:min-content;flex-shrink:0">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 40.0, 20.0)), ("b", (8.0, 8.0, 40.0, 20.0))];
    a_min_content_abspos_block_counts_a_max_content_child_up_to_its_max_width:
        r#"<div id=a style="position:absolute;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_float_counts_a_max_content_child_up_to_its_max_width:
        r#"<div style="width:300px"><div id=a style="float:left;width:min-content"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_min_content_multicol_container_repeats_the_max_content_child_in_its_columns:
        r#"<div id=a style="display:block;width:min-content;columns:2"><div id=b style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 110.0, 20.0)), ("b", (8.0, 8.0, 50.0, 20.0))];
    a_height_does_not_change_the_max_content_contribution:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50px;height:30px">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 30.0)), ("b", (8.0, 8.0, 50.0, 30.0))];
    a_percentage_max_width_does_not_limit_the_max_content_contribution:
        r#"<div id=a style="display:block;width:min-content"><div id=b style="width:max-content;max-width:50%">AAAA BBBB</div></div>"# =>
        [("a", (8.0, 8.0, 90.0, 20.0)), ("b", (8.0, 8.0, 45.0, 20.0))];
    a_min_content_table_adds_its_longest_word_to_a_max_content_block:
        r#"<div id=e1 style="width:max-content"><table id=e2 style="width:min-content;border-spacing:0;"><tr id=e3><td id=e4 style="padding:0">AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("e1", (8.0, 8.0, 40.0, 30.0)), ("e2", (8.0, 8.0, 40.0, 30.0)), ("e3", (8.0, 8.0, 40.0, 30.0)), ("e4", (8.0, 8.0, 40.0, 30.0))];
    a_max_content_table_adds_its_whole_line_to_a_min_content_block:
        r#"<div id=e1 style="width:min-content"><table id=e2 style="width:max-content;border-spacing:0;"><tr id=e3><td id=e4 style="padding:0">AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("e1", (8.0, 8.0, 140.0, 10.0)), ("e2", (8.0, 8.0, 140.0, 10.0)), ("e3", (8.0, 8.0, 140.0, 10.0)), ("e4", (8.0, 8.0, 140.0, 10.0))];
    a_min_content_table_adds_its_longest_word_to_an_inline_block:
        r#"<span id=e1 style="display:inline-block"><table id=e2 style="width:min-content;border-spacing:0;"><tr id=e3><td id=e4 style="padding:0">AAAA BBBB CCCC</td></tr></table></span>"# =>
        [("e1", (8.0, 8.0, 40.0, 30.0)), ("e2", (8.0, 8.0, 40.0, 30.0)), ("e3", (8.0, 8.0, 40.0, 30.0)), ("e4", (8.0, 8.0, 40.0, 30.0))];
    a_max_content_table_counts_up_to_its_max_width_in_a_min_content_row_container:
        r#"<div id=e1 style="width:min-content;display:flex"><table id=e2 style="width:max-content;border-spacing:0;max-width:70px"><tr id=e3><td id=e4 style="padding:0">AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("e1", (8.0, 8.0, 70.0, 30.0)), ("e2", (8.0, 8.0, 70.0, 30.0)), ("e3", (8.0, 8.0, 70.0, 30.0)), ("e4", (8.0, 8.0, 70.0, 30.0))];
    a_min_content_table_adds_its_longest_word_to_a_float:
        r#"<div id=e1 style="float:left"><table id=e2 style="width:min-content;border-spacing:0;"><tr id=e3><td id=e4 style="padding:0">AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("e1", (8.0, 8.0, 40.0, 30.0)), ("e2", (8.0, 8.0, 40.0, 30.0)), ("e3", (8.0, 8.0, 40.0, 30.0)), ("e4", (8.0, 8.0, 40.0, 30.0))];
    a_min_content_cell_keeps_its_longest_word_in_a_table:
        r#"<table id=e1 style="border-spacing:0"><tr id=e2><td id=e3 style="padding:0;width:min-content">AAAA BBBB CCCC</td><td id=e4 style="padding:0">XX</td></tr></table>"# =>
        [("e1", (8.0, 8.0, 160.0, 10.0)), ("e2", (8.0, 8.0, 160.0, 10.0)), ("e3", (8.0, 8.0, 140.0, 10.0)), ("e4", (148.0, 8.0, 20.0, 10.0))];
    a_max_content_cell_adds_its_whole_line_to_a_min_content_block:
        r#"<div id=e1 style="width:min-content"><table id=e2 style="border-spacing:0"><tr id=e3><td id=e4 style="padding:0;width:max-content">AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("e1", (8.0, 8.0, 40.0, 30.0)), ("e2", (8.0, 8.0, 40.0, 30.0)), ("e3", (8.0, 8.0, 40.0, 30.0)), ("e4", (8.0, 8.0, 40.0, 30.0))];
    a_min_content_cell_adds_its_whole_line_to_a_max_content_block:
        r#"<div id=e1 style="width:max-content"><table id=e2 style="border-spacing:0"><tr id=e3><td id=e4 style="padding:0;width:min-content">AAAA BBBB CCCC</td></tr></table></div>"# =>
        [("e1", (8.0, 8.0, 140.0, 10.0)), ("e2", (8.0, 8.0, 140.0, 10.0)), ("e3", (8.0, 8.0, 140.0, 10.0)), ("e4", (8.0, 8.0, 140.0, 10.0))];
    a_horizontal_child_of_a_vertical_block_counts_up_to_its_max_width:
        r#"<div id=e1 style="writing-mode:vertical-rl;display:inline-block"><div id=e2 style="writing-mode:horizontal-tb;width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("e1", (8.0, 8.0, 50.0, 20.0)), ("e2", (8.0, 8.0, 50.0, 20.0))];
    a_child_of_a_vertical_block_counts_up_to_its_max_width:
        r#"<div id=e1 style="writing-mode:vertical-rl;display:inline-block"><div id=e2 style="width:max-content;max-width:50px">AAAA BBBB</div></div>"# =>
        [("e1", (8.0, 8.0, 10.0, 90.0)), ("e2", (8.0, 8.0, 10.0, 90.0))];
    a_vertical_child_with_a_max_content_height_counts_up_to_its_max_height:
        r#"<div id=e1 style="display:inline-block"><div id=e2 style="writing-mode:vertical-rl;height:max-content;max-height:50px">AAAA BBBB</div></div>"# =>
        [("e1", (8.0, 8.0, 20.0, 50.0)), ("e2", (8.0, 8.0, 20.0, 50.0))];
}
