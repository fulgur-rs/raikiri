//! Balanced columns of an auto-height multi-column container do not hold the
//! margin below their last box: the end of a column truncates it (CSS
//! Fragmentation 3 §5.2). As in Chromium, the columns are still at least as
//! tall as the content with its margins divided among them. The heights are
//! the ones Chromium 153 gives for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf");

/// The border boxes `(x, y, width, height)` of the fragments of the element
/// with `id` on the first page.
fn rects(body: &str, id: &str) -> Vec<(f32, f32, f32, f32)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let document = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("layout must complete")
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .map(|fragment| {
            let r = fragment.rect();
            (r.x, r.y, r.width, r.height)
        })
        .collect()
}

/// The height of the container `m` of two columns 50px wide around
/// `content`, with `style` added, and the block offset of the line after it.
fn height(style: &str, content: &str) -> (f32, f32) {
    let body = format!(
        "<div id=m style='width:100px;columns:2;column-gap:0;{style}'>{content}</div>\
         <div id=n>n</div>"
    );
    (rects(&body, "m")[0].3, rects(&body, "n")[0].1)
}

#[test]
fn the_margin_below_the_last_paragraph_is_truncated() {
    let one = "<div style='margin-bottom:10px'>a</div>";
    assert_eq!(height("", one), (10.0, 10.0));
    // Padding below the columns still counts.
    assert_eq!(height("padding-bottom:5px", one), (15.0, 15.0));
    let three = "<div>a</div><div>b</div><div style='margin-bottom:10px'>c</div>";
    assert_eq!(height("", three), (20.0, 20.0));
}

#[test]
fn margins_still_count_in_the_height_divided_among_the_columns() {
    // 30px with the margin, divided in two.
    let between = "<div id=a style='margin-bottom:10px'>a</div><div id=b>b</div>";
    assert_eq!(height("", between), (15.0, 15.0));
    let body = format!("<div id=m style='width:100px;columns:2;column-gap:0'>{between}</div>");
    assert_eq!(rects(&body, "b"), vec![(50.0, 0.0, 50.0, 10.0)]);
    let wide = "<div id=a style='margin-bottom:20px'>a</div><div id=b>b</div>";
    assert_eq!(height("", wide), (20.0, 20.0));
}

#[test]
fn boxes_without_lines_truncate_the_last_margin_too() {
    let one = "<div style='height:10px;margin-bottom:10px'></div>";
    assert_eq!(height("", one), (10.0, 10.0));
    // 50px with the margin, divided in two, holds both boxes in the first
    // column.
    let two = "<div id=a style='height:10px'></div>\
               <div id=b style='height:10px;margin-bottom:30px'></div>";
    assert_eq!(height("", two), (25.0, 25.0));
    let body = format!("<div id=m style='width:100px;columns:2;column-gap:0'>{two}</div>");
    assert_eq!(rects(&body, "b"), vec![(0.0, 10.0, 50.0, 10.0)]);
    let gap = "<div style='height:10px;margin-bottom:20px'></div><div style='height:10px'></div>";
    assert_eq!(height("", gap), (20.0, 20.0));
    // The margin at the break is truncated, but counts in what is divided.
    let avoid = "<div style='height:40px;break-inside:avoid;margin-bottom:20px'></div>\
                 <div style='height:40px;break-inside:avoid;margin-top:10px'></div>";
    assert_eq!(height("", avoid), (50.0, 50.0));
}

#[test]
fn a_negative_trailing_margin_is_truncated_as_well() {
    let one = "<div style='margin-bottom:-5px'>a</div>";
    assert_eq!(height("", one), (10.0, 10.0));
}

#[test]
fn a_float_and_a_spanner_end_the_columns_as_well() {
    let float = "<div style='float:left;width:10px;height:10px;margin-bottom:10px'></div>";
    assert_eq!(height("", float), (10.0, 10.0));
    // A line beside the float does not keep the float's margin either.
    let beside = "<div style='float:left;width:10px;height:10px;margin-bottom:10px'></div>\
                  <div>a</div>";
    assert_eq!(height("", beside), (10.0, 10.0));
    let spanner = "<div style='margin-bottom:10px'>a</div><div style='column-span:all'>s</div>";
    assert_eq!(height("", spanner), (20.0, 20.0));
    // Columns filled in turn after the last spanner keep the margin.
    let after =
        "<div style='column-span:all'>s</div><div style='height:10px;margin-bottom:10px'></div>";
    assert_eq!(height("column-fill:auto", after), (30.0, 30.0));
    assert_eq!(height("", after), (20.0, 20.0));
}

#[test]
fn a_monolithic_float_keeps_its_margin_box_whole() {
    let sized =
        "<div style='float:left;width:10px;contain:size;height:10px;margin-bottom:10px'></div>";
    assert_eq!(height("", sized), (20.0, 20.0));
    let image = "<img style='float:left;width:10px;height:20px;margin-bottom:40px'>";
    assert_eq!(height("", image), (60.0, 60.0));
    let beside = "<div style='float:left;width:10px;contain:size;height:10px;margin-bottom:10px'></div>\
                  <div>a</div>";
    assert_eq!(height("", beside), (20.0, 20.0));
    // A float that may break still has its margin truncated.
    let avoid = "<div style='float:left;width:10px;height:20px;break-inside:avoid;margin-bottom:40px'></div>";
    assert_eq!(height("", avoid), (30.0, 30.0));
}

#[test]
fn the_columns_hold_at_least_the_stream_divided_among_them() {
    // 20px of box and 40px of margin below it, divided in two.
    let sized = "<div style='contain:size;height:20px;margin-bottom:40px'></div>";
    assert_eq!(height("", sized), (30.0, 30.0));
    let float = "<div style='float:left;width:10px;height:20px;margin-bottom:40px'></div>";
    assert_eq!(height("", float), (30.0, 30.0));
    // The margins between two boxes collapse to one in the stream: 80px.
    let collapsed = "<div style='contain:size;height:10px;margin-bottom:30px'></div>\
                     <div style='contain:size;height:10px;margin-top:30px;margin-bottom:30px'></div>";
    assert_eq!(height("", collapsed), (40.0, 40.0));
}

#[test]
fn margins_collapse_through_a_wrapper_in_a_column() {
    // The margin below the last child leaves the wrapper and is truncated
    // with it at the end of the column, the wrapper's own margin too.
    let wrapped = "<div id=w><div style='margin-bottom:10px'>a</div></div>";
    assert_eq!(height("", wrapped), (10.0, 10.0));
    let body = format!("<div id=m style='width:100px;columns:2;column-gap:0'>{wrapped}</div>");
    assert_eq!(rects(&body, "w"), vec![(0.0, 0.0, 50.0, 10.0)]);
    let own = "<div id=w style='margin-bottom:7px'><div style='margin-bottom:10px'>a</div></div>";
    assert_eq!(height("", own), (10.0, 10.0));
    // Nested wrappers pass it on as well.
    let nested = "<div><div><div style='margin-bottom:20px'>a</div></div></div>";
    assert_eq!(height("", nested), (15.0, 15.0));
}
