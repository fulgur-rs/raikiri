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
}

#[test]
fn a_float_and_a_spanner_end_the_columns_as_well() {
    let float = "<div style='float:left;width:10px;height:10px;margin-bottom:10px'></div>";
    assert_eq!(height("", float), (10.0, 10.0));
    let spanner = "<div style='margin-bottom:10px'>a</div><div style='column-span:all'>s</div>";
    assert_eq!(height("", spanner), (20.0, 20.0));
}
