//! A multi-column container stretched by an auto-height flex line takes the
//! height of the line, as any stretched flex item does (CSS Flexbox 1 §9.4),
//! while its columns balance to their content. `align-content` places that
//! content within the stretched height. The values are the ones Chromium 153
//! gives for the same documents.

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

/// A flex line made 200px tall by a sibling, holding the multi-column
/// container `m` of two columns 50px wide around `content`, with `style`
/// added.
fn stretched(height: u32, style: &str, content: &str) -> String {
    format!(
        "<div style='display:flex'><div style='height:{height}px;width:10px'></div>\
         <div id=m style='width:100px;columns:2;column-gap:0;{style}'>{content}</div></div>"
    )
}

#[test]
fn a_stretched_container_takes_the_height_of_the_flex_line() {
    let body = stretched(200, "", "<div id=a>a</div>");
    assert_eq!(rects(&body, "m"), vec![(10.0, 0.0, 100.0, 200.0)]);
    // The columns still balance to their content.
    let body = stretched(200, "", "<div id=a>a b c d</div>");
    assert_eq!(rects(&body, "m"), vec![(10.0, 0.0, 100.0, 200.0)]);
    assert_eq!(rects(&body, "a"), vec![(10.0, 0.0, 50.0, 20.0)]);
    // Padding and borders are inside the stretched border box.
    let body = stretched(
        200,
        "padding:5px;border:3px solid",
        "<div id=a>a b c d</div>",
    );
    assert_eq!(rects(&body, "m"), vec![(10.0, 0.0, 100.0, 200.0)]);
    assert_eq!(rects(&body, "a"), vec![(18.0, 8.0, 42.0, 20.0)]);
}

#[test]
fn a_container_that_is_not_stretched_keeps_its_content_height() {
    let body = "<div style='display:flex;align-items:start'><div style='height:200px;width:10px'></div>\
                <div id=m style='width:100px;columns:2;column-gap:0'><div>a</div></div></div>";
    assert_eq!(rects(body, "m"), vec![(10.0, 0.0, 100.0, 10.0)]);
    // A line its own content makes taller than the sibling measures the
    // container as one column across its whole width; the container still
    // balances its columns and takes their height.
    let four = "<div>a</div><div>b</div><div>c</div><div id=d>d</div>";
    let body = stretched(5, "", four);
    assert_eq!(rects(&body, "m"), vec![(10.0, 0.0, 100.0, 20.0)]);
    assert_eq!(rects(&body, "d"), vec![(60.0, 10.0, 50.0, 10.0)]);
    let boxes = "<div style='height:30px'></div><div style='height:30px'></div>";
    let body = stretched(5, "", boxes);
    assert_eq!(rects(&body, "m"), vec![(10.0, 0.0, 100.0, 30.0)]);
    // A definite line keeps the container at its height even when the
    // content as one column is just as tall.
    let body = format!(
        "<div style='display:flex;height:40px'>\
         <div id=m style='width:100px;columns:2;column-gap:0'>{four}</div></div>"
    );
    assert_eq!(rects(&body, "m"), vec![(0.0, 0.0, 100.0, 40.0)]);
}

#[test]
fn content_alignment_uses_the_stretched_height() {
    let body = stretched(200, "align-content:center", "<div id=a>a b c d</div>");
    assert_eq!(rects(&body, "a"), vec![(10.0, 90.0, 50.0, 20.0)]);
    let body = stretched(200, "align-content:end", "<div id=a>a</div>");
    assert_eq!(rects(&body, "a"), vec![(10.0, 190.0, 50.0, 10.0)]);
    // Boxes without lines.
    let boxes = "<div id=a style='height:30px'></div><div id=b style='height:30px'></div>";
    let body = stretched(100, "align-content:center", boxes);
    assert_eq!(rects(&body, "m"), vec![(10.0, 0.0, 100.0, 100.0)]);
    assert_eq!(rects(&body, "a"), vec![(10.0, 35.0, 50.0, 30.0)]);
    assert_eq!(rects(&body, "b"), vec![(60.0, 35.0, 50.0, 30.0)]);
    let body = stretched(
        100,
        "align-content:center",
        "<div id=a style='height:30px'></div>",
    );
    assert_eq!(
        rects(&body, "a"),
        vec![(10.0, 42.5, 50.0, 15.0), (60.0, 42.5, 50.0, 15.0)]
    );
}

#[test]
fn a_spanner_sits_below_the_balanced_columns_of_a_stretched_container() {
    let body = stretched(
        200,
        "",
        "<div id=a>a b</div><div id=s style='column-span:all'>s</div><div id=b>c d</div>",
    );
    assert_eq!(rects(&body, "m"), vec![(10.0, 0.0, 100.0, 200.0)]);
    assert_eq!(rects(&body, "s"), vec![(10.0, 10.0, 100.0, 10.0)]);
    assert_eq!(rects(&body, "b"), vec![(10.0, 20.0, 50.0, 10.0)]);
}
