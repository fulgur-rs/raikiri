//! A flex item with an `aspect-ratio` that states its main size and leaves its
//! cross size `auto` takes the cross size from the main size it ends up with
//! after flexing, not from the one it states: a `20px` wide square that grows to
//! `100px` is `100px` high, and a `20px` high one that grows to `100px` in a
//! column is `100px` wide. This holds for the items of every kind of box: a
//! block, a paragraph, a flex or grid container.
//!
//! * Content raises the cross size above the ratio size unless the item scrolls
//!   or states a `min-*` size on that axis.
//! * An item that does not flex, or that states both its sizes, keeps the size
//!   it states.
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
    an_empty_flex_container_item_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px"><div id=a style="display:flex;width:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_paragraph_item_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    an_empty_flex_container_item_of_a_column_takes_its_width_from_the_flexed_height:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_paragraph_item_of_a_column_takes_its_width_from_the_flexed_height:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="height:20px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_flex_container_item_that_does_not_flex_keeps_the_ratio_size_it_states:
        r#"<div style="display:flex;width:100px"><div id=a style="display:flex;width:50px;aspect-ratio:1"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 50.0))];
    a_paragraph_item_that_does_not_flex_keeps_the_ratio_size_it_states:
        r#"<div style="display:flex;width:100px"><div id=a style="width:50px;aspect-ratio:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 50.0))];
    a_flex_container_item_with_padding_that_does_not_flex_keeps_the_ratio_size:
        r#"<div style="display:flex;width:100px"><div id=a style="display:flex;width:50px;padding:5px;aspect-ratio:1"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 50.0))];
    a_flex_container_item_with_a_tall_ratio_that_does_not_flex_keeps_the_ratio_size:
        r#"<div style="display:flex;width:100px"><div id=a style="display:flex;width:50px;aspect-ratio:1/2"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 100.0))];
    a_flex_container_item_that_shrinks_takes_its_height_from_the_shrunk_width:
        r#"<div style="display:flex;width:100px"><div id=a style="display:flex;width:150px;aspect-ratio:1;flex-shrink:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_paragraph_item_that_shrinks_takes_its_height_from_the_shrunk_width:
        r#"<div style="display:flex;width:100px"><div id=a style="width:150px;aspect-ratio:1;flex-shrink:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_tall_ratio_flex_container_item_takes_twice_the_flexed_width_as_height:
        r#"<div style="display:flex;width:100px"><div id=a style="display:flex;width:20px;aspect-ratio:1/2;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 200.0))];
    a_wide_ratio_flex_container_item_takes_half_the_flexed_width_as_height:
        r#"<div style="display:flex;width:100px"><div id=a style="display:flex;width:20px;aspect-ratio:2/1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 50.0))];
    a_block_item_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px"><div id=a style="display:block;width:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_grid_item_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px"><div id=a style="display:grid;width:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_reversed_row_flex_container_item_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px;flex-direction:row-reverse"><div id=a style="display:flex;width:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_flex_container_item_that_scrolls_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px"><div id=a style="display:flex;width:20px;aspect-ratio:1;flex-grow:1;overflow:hidden"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    content_raises_the_height_of_a_flex_container_item_above_the_ratio_height:
        r#"<div style="display:flex;width:100px"><div id=a style="display:flex;width:20px;aspect-ratio:1;flex-grow:1"><div style="width:10px;height:150px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 150.0))];
    a_start_aligned_flex_container_item_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=a style="display:flex;width:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_stretched_flex_container_item_with_a_tall_row_keeps_the_stretched_height:
        r#"<div style="display:flex;width:100px;height:300px"><div id=a style="display:flex;width:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 300.0))];
    a_flex_container_item_of_a_column_that_does_not_flex_keeps_the_ratio_size:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:50px;aspect-ratio:1"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 50.0))];
    content_raises_the_width_of_a_flex_container_item_of_a_column:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1"><div style="width:150px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 150.0, 100.0))];
    a_scrolling_flex_container_item_of_a_column_is_not_raised_by_its_content:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1;overflow:hidden"><div style="width:150px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    padding_of_a_border_box_column_item_stays_inside_the_ratio_width:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1;padding:5px"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    padding_of_a_content_box_column_item_adds_to_the_ratio_width:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1;padding:5px;box-sizing:content-box"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_max_width_caps_the_width_of_a_flexed_column_item:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1;max-width:60px"></div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 100.0))];
    a_tall_ratio_gives_a_narrower_flexed_column_item:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1/2;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 100.0))];
    a_column_item_that_shrinks_takes_its_width_from_the_shrunk_height:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:150px;aspect-ratio:1;flex-shrink:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_stretched_column_item_keeps_the_width_the_container_gives:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 100.0))];
    a_stretched_paragraph_item_of_a_column_keeps_the_width_the_container_gives:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px"><div id=a style="height:20px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 100.0))];
    a_block_item_of_a_column_takes_its_width_from_the_flexed_height:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="height:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_grid_item_of_a_column_takes_its_width_from_the_flexed_height:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:grid;height:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_min_width_raises_the_width_of_a_flexed_column_item:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1;min-width:150px"></div></div>"# =>
        [("a", (8.0, 8.0, 150.0, 100.0))];
    a_zero_min_width_keeps_content_from_raising_a_flexed_column_item:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1;min-width:0"><div style="width:150px;height:10px"></div></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_column_container_without_a_height_leaves_the_item_at_its_stated_height:
        r#"<div style="display:flex;flex-direction:column;width:300px"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 300.0, 20.0))];
    a_reversed_column_item_takes_its_width_from_the_flexed_height:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start;flex-direction:column-reverse"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    an_item_that_states_both_sizes_ignores_the_flexed_ratio:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;width:30px;height:20px;aspect-ratio:1;flex-grow:1"></div></div>"# =>
        [("a", (8.0, 8.0, 30.0, 100.0))];
    a_max_height_caps_the_flexed_height_of_a_column_item:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="display:flex;height:20px;aspect-ratio:1;flex-grow:1;max-height:60px"></div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 60.0))];
    a_max_width_caps_a_flexed_paragraph_item_of_a_column:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="height:20px;aspect-ratio:1;flex-grow:1;max-width:60px">x</div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 100.0))];
    content_raises_the_width_of_a_flexed_paragraph_item_of_a_column:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="height:20px;aspect-ratio:1;flex-grow:1">aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_scrolling_paragraph_item_of_a_column_is_not_raised_by_its_content:
        r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=a style="height:20px;aspect-ratio:1;flex-grow:1;overflow:hidden">aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    content_raises_the_height_of_a_flexed_paragraph_item:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1">aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 140.0))];
    a_scrolling_paragraph_item_keeps_the_ratio_height_whatever_the_content:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1;overflow:hidden">aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_zero_min_height_keeps_content_from_raising_a_flexed_paragraph_item:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1;min-height:0">aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_max_height_caps_a_flexed_paragraph_item:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1;max-height:60px">aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa aa</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 60.0))];
    padding_of_a_border_box_paragraph_item_stays_inside_the_ratio_height:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1;padding:5px">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    padding_of_a_content_box_paragraph_item_adds_to_the_ratio_height:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1;padding:5px;box-sizing:content-box">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_wrapping_container_flexes_a_paragraph_item_the_same_way:
        r#"<div style="display:flex;width:100px;flex-wrap:wrap"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_reversed_row_paragraph_item_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px;flex-direction:row-reverse"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    an_rtl_container_flexes_a_paragraph_item_the_same_way:
        r#"<div style="display:flex;width:100px;direction:rtl"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_paragraph_item_that_states_both_sizes_keeps_its_stated_height:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;height:30px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 30.0))];
    a_min_height_raises_a_flexed_paragraph_item:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1;min-height:150px">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 150.0))];
    a_max_width_caps_a_flexed_paragraph_item:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1;max-width:60px">x</div></div>"# =>
        [("a", (8.0, 8.0, 60.0, 60.0))];
    a_paragraph_item_with_a_zero_flex_basis_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1;flex-basis:0">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_paragraph_item_shrunk_to_a_zero_minimum_takes_its_height_from_the_shrunk_width:
        r#"<div style="display:flex;width:100px"><div id=a style="width:200px;aspect-ratio:1;flex-shrink:1;min-width:0">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    an_inline_flex_container_flexes_a_paragraph_item_the_same_way:
        r#"<div style="display:inline-flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_centered_paragraph_item_takes_its_height_from_the_flexed_width:
        r#"<div style="display:flex;width:100px;align-items:center"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 100.0))];
    a_centered_paragraph_item_of_a_tall_row_is_not_stretched:
        r#"<div style="display:flex;width:100px;height:300px;align-items:center"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 108.0, 100.0, 100.0))];
    two_paragraph_items_flex_to_equal_ratio_boxes:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1">x</div><div id=b style="width:20px;aspect-ratio:1;flex-grow:1">y</div></div>"# =>
        [("a", (8.0, 8.0, 50.0, 50.0)), ("b", (58.0, 8.0, 50.0, 50.0))];
    a_tall_ratio_paragraph_item_takes_twice_the_flexed_width_as_height:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1/2;flex-grow:1">x</div></div>"# =>
        [("a", (8.0, 8.0, 100.0, 200.0))];
    margins_of_a_paragraph_item_stay_outside_its_ratio_box:
        r#"<div style="display:flex;width:100px"><div id=a style="width:20px;aspect-ratio:1;flex-grow:1;margin:5px">x</div></div>"# =>
        [("a", (13.0, 13.0, 90.0, 90.0))];
}
